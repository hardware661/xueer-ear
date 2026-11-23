//! 一轮听写的云端识别：按句（本地断句出来的一段）连一次云端。设置里选主通道（豆包或微信），另一家是备用：
//! 每句先连主通道，1.5 秒还没连好就同时连备用，谁先好用谁；主通道出错、被拒或中途断了，这句录好的音整句
//! 改发备用；这一轮主通道连着失败 3 次，剩下的话都走备用，下一轮再试主通道。
//! 为了一开口就有字，开麦后、每句说完后都在后台先把下一句要用的连好；豆包的会话空等 15 秒还能用、
//! 30 秒就被服务端断了，所以预先连好的超过 10 秒没用上就重连。
use super::{Audio, Capture, Counts, Event, Source, doubao_turn, wetype};
use crate::{
    Result,
    race::{self, Won},
    replay::Replay,
    settings::Engine,
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::TryRecvError,
    },
    thread,
    time::{Duration, Instant},
};

/// 主通道多久没连好就同时连备用
const PATIENCE: Duration = Duration::from_millis(1500);
/// 预先连好的放多久就不用了
const WARM_MAX_AGE: Duration = Duration::from_secs(10);
/// 这一轮主通道连着失败几次后不再试
const GIVE_UP: u32 = 3;

/// 两家云端
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Doubao,
    WeType,
}

impl Kind {
    fn of(engine: Engine) -> Self {
        match engine {
            Engine::Doubao => Self::Doubao,
            Engine::Wetype => Self::WeType,
        }
    }

    fn other(self) -> Self {
        match self {
            Self::Doubao => Self::WeType,
            Self::WeType => Self::Doubao,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Doubao => "doubao",
            Self::WeType => "wetype",
        }
    }

    fn chinese(self) -> &'static str {
        match self {
            Self::Doubao => "豆包",
            Self::WeType => "微信",
        }
    }

    /// 连这一家（阻塞）；被取消返回 None
    fn connect(self, stop: &AtomicBool, context: Option<&str>) -> Result<Option<Turn>> {
        Ok(match self {
            Self::Doubao => doubao_turn::Turn::start(stop, context)?.map(Turn::Doubao),
            Self::WeType => wetype::Turn::start(stop)?.map(Turn::WeType),
        })
    }

    /// 在后台线程里连这一家
    fn spawn(self, stop: &Arc<AtomicBool>, context: &Option<String>) -> race::Pending<Turn> {
        let (stop, context) = (stop.clone(), context.clone());
        race::spawn(move || self.connect(&stop, context.as_deref()))
    }
}

/// 一句话正在用的那条连接
enum Turn {
    Doubao(doubao_turn::Turn),
    WeType(wetype::Turn),
}

impl Turn {
    fn upload(&mut self, chunk: &[u8]) -> Result<()> {
        match self {
            Self::Doubao(turn) => turn.upload(chunk),
            Self::WeType(turn) => turn.upload(chunk),
        }
    }

    fn poll(
        &mut self,
        stop: &AtomicBool,
        counts: &mut Counts,
        output: &mut impl FnMut(Event) -> Result<()>,
    ) -> Result<()> {
        match self {
            Self::Doubao(turn) => turn.poll(stop, counts, output),
            Self::WeType(turn) => turn.poll(stop, counts, output),
        }
    }

    fn finish(
        self,
        stop: &AtomicBool,
        counts: &mut Counts,
        output: &mut impl FnMut(Event) -> Result<()>,
    ) -> Result<String> {
        match self {
            Self::Doubao(turn) => turn.finish(stop, counts, output),
            Self::WeType(turn) => turn.finish(stop, counts, output),
        }
    }

    fn kind(&self) -> Kind {
        match self {
            Self::Doubao(_) => Kind::Doubao,
            Self::WeType(_) => Kind::WeType,
        }
    }
}

/// 这一轮走哪家
struct Channel {
    primary: Kind,
    /// 主通道连着失败的次数（主通道好好说完一句就清零）
    failures: u32,
    /// 带给豆包的热词提示（这一轮开始时读一次）
    context: Option<String>,
    /// 后台预先在连的下一句：哪家、什么时候开始连的
    warm: Option<(Kind, Instant, race::Pending<Turn>)>,
}

impl Channel {
    fn new(engine: Engine, context: Option<String>) -> Self {
        Self {
            primary: Kind::of(engine),
            failures: 0,
            context,
            warm: None,
        }
    }

    /// 这一句先连哪家：主通道这一轮失败太多次就直接用备用
    fn first(&self) -> Kind {
        if self.failures < GIVE_UP {
            self.primary
        } else {
            self.primary.other()
        }
    }

    /// 在后台先把下一句要用的连好；已经有一个新鲜的就不重复连
    fn prepare(&mut self, stop: &Arc<AtomicBool>) {
        let kind = self.first();
        let fresh = self
            .warm
            .as_ref()
            .is_some_and(|(k, since, _)| *k == kind && since.elapsed() < WARM_MAX_AGE);
        if !fresh && !stop.load(Ordering::Acquire) {
            self.warm = Some((kind, Instant::now(), kind.spawn(stop, &self.context)));
        }
    }

    /// 新的一句：用预先连好的（没有就现连）；主通道慢或出错就抢着连备用，告诉面板这句走了哪家、等了多久
    fn start(
        &mut self,
        stop: &Arc<AtomicBool>,
        output: &mut impl FnMut(Event) -> Result<()>,
    ) -> Result<Option<Turn>> {
        let began = Instant::now();
        let kind = self.first();
        let pending = match self.warm.take() {
            Some((k, since, pending)) if k == kind && since.elapsed() < WARM_MAX_AGE => pending,
            _ => kind.spawn(stop, &self.context),
        };
        let cancelled = || stop.load(Ordering::Acquire);
        let won = if kind == self.primary {
            let (backup, context) = (kind.other(), &self.context);
            race::first_ready(
                pending,
                || backup.spawn(stop, context),
                PATIENCE,
                &cancelled,
            )?
        } else {
            race::wait(pending, &cancelled)?.map(|turn| Won {
                side: race::Side::Primary,
                turn,
                primary_miss: None,
            })
        };
        let Some(won) = won else {
            return Ok(None);
        };
        let used = won.turn.kind();
        if let Some(reason) = &won.primary_miss {
            self.failures += 1;
            eprintln!(
                "{}没连上，这句改用{}：{reason}",
                self.primary.chinese(),
                used.chinese()
            );
        }
        output(Event::Channel {
            name: used.name(),
            fallback: used != self.primary,
            waited_ms: began.elapsed().as_millis().min(u128::from(u32::MAX)) as u32,
        })?;
        Ok(Some(won.turn))
    }

    /// 这一句用 `from` 出错了：换哪家重发（主通道出的错换备用；备用出的错换回主通道，主通道已放弃就还用备用）
    fn retry_with(&mut self, from: Kind, error: &dyn std::error::Error) -> Kind {
        let to = if from == self.primary || self.failures < GIVE_UP {
            from.other()
        } else {
            from
        };
        if from == self.primary {
            self.failures += 1;
        }
        eprintln!(
            "{}出错，这句改用{}重发：{error}",
            from.chinese(),
            to.chinese()
        );
        to
    }

    /// 主通道好好说完了一句
    fn succeeded(&mut self, kind: Kind) {
        if kind == self.primary {
            self.failures = 0;
        }
    }
}

/// 这一句的连接出错了：还没重试过就换一家（见 `Channel::retry_with`）连上，并把这句已经录的音重发一遍；
/// 重试过了就把原错往外抛。重连前发一个 `reconnecting` 活动事件，让面板能显示出来。
fn reconnect(
    stop: &AtomicBool,
    channel: &mut Channel,
    replay: &mut Replay,
    from: Kind,
    error: Box<dyn std::error::Error + Send + Sync>,
    output: &mut impl FnMut(Event) -> Result<()>,
) -> Result<Option<Turn>> {
    if replay.retried() {
        return Err(error);
    }
    let to = channel.retry_with(from, &*error);
    output(Event::Activity("reconnecting"))?;
    let began = Instant::now();
    let context = channel.context.clone();
    let turn = replay.recover(
        error,
        || to.connect(stop, context.as_deref()),
        |turn, chunk| turn.upload(chunk),
    )?;
    if let Some(turn) = turn.as_ref() {
        output(Event::Channel {
            name: turn.kind().name(),
            fallback: turn.kind() != channel.primary,
            waited_ms: began.elapsed().as_millis().min(u128::from(u32::MAX)) as u32,
        })?;
        output(Event::Activity("recognizing"))?;
    }
    Ok(turn)
}

/// 主人的热词（数据目录里的热词表，按现在的设置要不要带自带表）；读不到就没有提示
fn hints() -> Vec<String> {
    let settings = crate::settings::Settings::load_default().unwrap_or_default();
    crate::settings::Settings::data_dir()
        .map(|dir| crate::postprocess::hint_words(&settings, &dir))
        .unwrap_or_default()
}

pub fn run(
    source: Source,
    engine: Engine,
    stop: Arc<AtomicBool>,
    mut output: impl FnMut(Event) -> Result<()>,
) -> Result<String> {
    let mut capture = Capture::start(source, stop.clone());
    // 热词表带给豆包当提示（豆包是主通道还是备用都带；这一轮开始时读一次）
    let mut channel = Channel::new(engine, crate::doubao::context(&hints()));
    let mut turn: Option<Turn> = None;
    let mut counts = Counts::default();
    let mut ready = false;
    let mut text = String::new();
    let mut last_report = Instant::now();
    // 当前这句已上传的音频留底：连接断了重连后整句重发，每句只重试一次
    let mut replay = Replay::default();
    loop {
        match capture.input.try_recv() {
            Ok(Audio::Ready(device)) => {
                ready = true;
                if let Some(device) = device {
                    output(Event::Input {
                        name: device.name,
                        muted: device.muted,
                        volume: device.volume,
                    })?;
                }
                output(Event::Started)?;
                output(Event::Activity("armed"))?;
                channel.prepare(&stop);
            }
            Ok(Audio::Start) => {
                if turn.is_some() {
                    return Err("overlapping speech turns".into());
                }
                if !stop.load(Ordering::Acquire) {
                    output(Event::Activity("connecting"))?;
                    replay = Replay::default();
                    let started = match channel.start(&stop, &mut output) {
                        Ok(started) => started,
                        Err(error) => {
                            let first = channel.first();
                            reconnect(&stop, &mut channel, &mut replay, first, error, &mut output)?
                        }
                    };
                    if !stop.load(Ordering::Acquire) {
                        turn = started;
                        output(Event::Activity("recognizing"))?;
                    }
                }
            }
            Ok(Audio::Chunk(audio)) => {
                if !stop.load(Ordering::Acquire) {
                    replay.push(&audio);
                    if let Some(current) = turn.as_mut() {
                        if let Err(error) = current.upload(&audio) {
                            // replay 里已经含这一块，重连时会一起重发
                            let from = current.kind();
                            turn = reconnect(
                                &stop,
                                &mut channel,
                                &mut replay,
                                from,
                                error,
                                &mut output,
                            )?;
                        }
                        counts.pieces += 1;
                    }
                }
            }
            Ok(Audio::End) => {
                if let Some(ended) = turn.take() {
                    // 收句等最终结果时断线（最容易超时的几秒）：和说话中途断线一样，重连、整句重发、再收一次（审查 R8）
                    let from = ended.kind();
                    let mut sink_failed = false;
                    let finished = ended.finish(&stop, &mut counts, &mut |event| {
                        output(event).inspect_err(|_| sink_failed = true)
                    });
                    let part = match finished {
                        Ok(part) => {
                            channel.succeeded(from);
                            part
                        }
                        Err(error) if sink_failed => return Err(error),
                        Err(error) => match reconnect(
                            &stop,
                            &mut channel,
                            &mut replay,
                            from,
                            error,
                            &mut output,
                        )? {
                            Some(again) => again.finish(&stop, &mut counts, &mut output)?,
                            None => String::new(),
                        },
                    };
                    if text.len() + part.len() > (1 << 20) {
                        return Err("dictation text exceeds limit".into());
                    }
                    text.push_str(&part);
                    output(Event::Activity("paused"))?;
                    // 下一句先在后台连着
                    channel.prepare(&stop);
                }
            }
            Err(TryRecvError::Disconnected) => {
                capture.join()?;
                if !ready {
                    return Err("audio capture failed to initialize".into());
                }
                break;
            }
            Err(TryRecvError::Empty) => {
                if turn.is_none() {
                    if ready {
                        // 预先连好的放太久了就换个新的
                        channel.prepare(&stop);
                    }
                    thread::sleep(Duration::from_millis(10));
                }
            }
        }
        if !stop.load(Ordering::Acquire) {
            if let Some(current) = turn.as_mut() {
                // 只有连接层的错才重连；output 回调（上屏）出的错照常往外抛
                let mut sink_failed = false;
                let polled = current.poll(&stop, &mut counts, &mut |event| {
                    output(event).inspect_err(|_| sink_failed = true)
                });
                if let Err(error) = polled {
                    if sink_failed {
                        return Err(error);
                    }
                    let from = current.kind();
                    turn = reconnect(&stop, &mut channel, &mut replay, from, error, &mut output)?;
                }
            }
        }
        if last_report.elapsed() >= Duration::from_millis(250) {
            output(capture.signal.event(
                counts.pieces,
                counts.responses,
                counts.partials,
                counts.finals,
            ))?;
            last_report = Instant::now();
        }
    }
    output(capture.signal.event(
        counts.pieces,
        counts.responses,
        counts.partials,
        counts.finals,
    ))?;
    Ok(text)
}
