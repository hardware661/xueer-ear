//! 一轮听写的云端识别：按句（本地断句出来的一段）连一次云端。设置选了豆包就每句先走豆包，
//! 豆包连不上、被拒或中途断了，这句录好的音整句改发微信输入法；这一轮豆包连着失败 3 次，剩下的话都走微信，
//! 下一轮再试豆包。只选微信就一直走微信，断线时和以前一样重连重发一次。
use super::{Audio, Capture, Counts, Event, Source, doubao_turn, wetype};
use crate::{Result, replay::Replay, settings::Engine};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::TryRecvError,
    },
    thread,
    time::{Duration, Instant},
};

/// 这一轮豆包连着失败几次后不再试
const DOUBAO_GIVE_UP: u32 = 3;

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

    fn is_doubao(&self) -> bool {
        matches!(self, Self::Doubao(_))
    }
}

/// 这一轮走哪条
struct Channel {
    doubao: bool,
    /// 豆包连着失败的次数（成功一句就清零）
    failures: u32,
    /// 带给豆包的热词提示（这一轮开始时读一次）
    context: Option<String>,
}

impl Channel {
    fn new(engine: Engine, context: Option<String>) -> Self {
        Self {
            doubao: engine == Engine::Doubao,
            failures: 0,
            context,
        }
    }

    /// 新的一句：该走豆包就先试豆包，不行改走微信
    fn start(
        &mut self,
        stop: &AtomicBool,
        output: &mut impl FnMut(Event) -> Result<()>,
    ) -> Result<Option<Turn>> {
        if self.doubao && self.failures < DOUBAO_GIVE_UP {
            match doubao_turn::Turn::start(stop, self.context.as_deref()) {
                Ok(Some(turn)) => {
                    output(Event::Channel {
                        name: "doubao",
                        fallback: false,
                    })?;
                    return Ok(Some(Turn::Doubao(turn)));
                }
                Ok(None) => return Ok(None),
                Err(error) => self.fell_back(&*error, output)?,
            }
        }
        let turn = wetype::Turn::start(stop)?.map(Turn::WeType);
        if turn.is_some() && !self.doubao {
            output(Event::Channel {
                name: "wetype",
                fallback: false,
            })?;
        }
        Ok(turn)
    }

    /// 豆包这句不行了：记一次失败，告诉面板这句改用微信
    fn fell_back(
        &mut self,
        error: &dyn std::error::Error,
        output: &mut impl FnMut(Event) -> Result<()>,
    ) -> Result<()> {
        self.failures += 1;
        eprintln!("豆包没连上，这句改用微信：{error}");
        output(Event::Channel {
            name: "wetype",
            fallback: true,
        })
    }

    /// 豆包好好说完了一句
    fn succeeded(&mut self) {
        self.failures = 0;
    }
}

/// 这一句的连接出错了：还没重试过就重连，并把这句已经录的音重发一遍；重试过了就把原错往外抛。
/// 豆包出的错直接改走微信重发（同一台虚拟设备马上重试多半还是不行）；微信出的错就重连微信。
/// 重连前发一个 `reconnecting` 活动事件，让面板能显示出来。
fn reconnect(
    stop: &AtomicBool,
    channel: &mut Channel,
    replay: &mut Replay,
    from_doubao: bool,
    error: Box<dyn std::error::Error + Send + Sync>,
    output: &mut impl FnMut(Event) -> Result<()>,
) -> Result<Option<Turn>> {
    if replay.retried() {
        return Err(error);
    }
    if from_doubao {
        channel.fell_back(&*error, output)?;
    } else {
        eprintln!("识别连接出错，重连并重发这一句：{error}");
    }
    output(Event::Activity("reconnecting"))?;
    let turn = replay.recover(
        error,
        || Ok(wetype::Turn::start(stop)?.map(Turn::WeType)),
        |turn, chunk| turn.upload(chunk),
    )?;
    if turn.is_some() {
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
    // 走豆包时把热词表带过去当提示（这一轮开始时读一次）
    let context = (engine == Engine::Doubao)
        .then(hints)
        .and_then(|words| crate::doubao::context(&words));
    let mut channel = Channel::new(engine, context);
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
                            reconnect(&stop, &mut channel, &mut replay, false, error, &mut output)?
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
                            let from_doubao = current.is_doubao();
                            turn = reconnect(
                                &stop,
                                &mut channel,
                                &mut replay,
                                from_doubao,
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
                    let from_doubao = ended.is_doubao();
                    let mut sink_failed = false;
                    let finished = ended.finish(&stop, &mut counts, &mut |event| {
                        output(event).inspect_err(|_| sink_failed = true)
                    });
                    let part = match finished {
                        Ok(part) => {
                            if from_doubao {
                                channel.succeeded();
                            }
                            part
                        }
                        Err(error) if sink_failed => return Err(error),
                        Err(error) => match reconnect(
                            &stop,
                            &mut channel,
                            &mut replay,
                            from_doubao,
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
                    let from_doubao = current.is_doubao();
                    turn = reconnect(
                        &stop,
                        &mut channel,
                        &mut replay,
                        from_doubao,
                        error,
                        &mut output,
                    )?;
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
