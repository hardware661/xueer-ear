//! 翻译服务池（主人 2026-11-24：长文分段翻，每段各自计时；先用最快最稳的那家，慢了让下一家补）。
//!
//! - 先按「最近实测」排好先后（开始按华为测的：火山、微软、B 站、微信，配了 Key 的排后面），
//!   同一篇的每段都先交给排第一的那家，用词前后一致；
//! - 每段按长度给时间：短段 2 秒，每多 300 字多 1 秒，最多 8 秒。到时间还没回来，就让下一家也翻这一段，
//!   谁先回来用谁；
//! - 一家出错、回来的译文不能用（空的、方向不对、少了一截……）马上换下一家；一家最多等 20 秒；
//! - 一段要所有家都不行才算失败。整篇不设总时限：只要还有段在翻就接着等；
//! - 被限流的那家歇 5 分钟，Key 不对的歇 6 小时，连着失败 3 次的歇 1 分钟。
use crate::translate::{Service, Target};
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};

/// 一家没翻成的原因
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// 限流了（HTTP 429）
    Limited(String),
    /// Key 不对、没开通（HTTP 401/403）
    Denied(String),
    /// 别的：连不上、超时、出错、译文不能用
    Other(String),
}

impl Failure {
    pub fn reason(&self) -> &str {
        match self {
            Self::Limited(reason) | Self::Denied(reason) | Self::Other(reason) => reason,
        }
    }
}

/// 让某一家翻一段（同步等结果）
pub trait Translator: Send + Sync {
    fn translate(&self, service: Service, text: &str, target: Target) -> Result<String, Failure>;
}

/// 等多久
#[derive(Debug, Clone)]
pub struct Timing {
    /// 一段先给这么久，到时间没回来就让下一家也翻
    pub hedge_base: Duration,
    /// 每多 300 字多给这么久
    pub hedge_per_300: Duration,
    pub hedge_max: Duration,
    /// 一家最多等这么久
    pub attempt_limit: Duration,
}

impl Timing {
    pub const REAL: Self = Self {
        hedge_base: Duration::from_secs(2),
        hedge_per_300: Duration::from_secs(1),
        hedge_max: Duration::from_secs(8),
        attempt_limit: Duration::from_secs(20),
    };

    fn hedge(&self, chars: usize) -> Duration {
        (self.hedge_base + self.hedge_per_300 * u32::try_from(chars / 300).unwrap_or(u32::MAX))
            .min(self.hedge_max)
    }
}

/// 同时在翻几段（每段可能同时问着两家）
pub const PIECES_IN_FLIGHT: usize = 6;

/// 每段的结果：译文和哪家翻的；所有家都不行是 Err(各家的原因)
pub type PieceResult = Result<(String, Service), String>;

#[derive(Debug)]
pub struct Report {
    pub results: Vec<PieceResult>,
    /// 慢了让下一家补了几次
    pub hedges: usize,
}

/// 各家最近的表现（整个进程共用）
#[derive(Debug, Default)]
pub struct Stats {
    records: HashMap<Service, Record>,
}

#[derive(Debug, Clone, Copy)]
struct Record {
    /// 最近翻一段要多少毫秒（指数平均）
    ms: f64,
    /// 连着失败几次
    failures: u32,
    /// 歇到什么时候
    rest_until: Option<Instant>,
}

impl Stats {
    fn record(&mut self, service: Service) -> &mut Record {
        self.records.entry(service).or_insert(Record {
            ms: f64::from(service.expected_ms()),
            failures: 0,
            rest_until: None,
        })
    }

    /// 这次的先后：没在歇的按最近快慢排（失败多的往后挪）；都在歇就全拿来试。
    /// 歇够了的回到原来的位置：歇就是罚过了，不再按之前的失败往后挪
    pub fn order(&mut self, services: &[Service], now: Instant) -> Vec<Service> {
        let mut ready: Vec<(f64, usize, Service)> = Vec::new();
        for (i, &service) in services.iter().enumerate() {
            let record = self.record(service);
            match record.rest_until {
                Some(until) if until > now => continue,
                Some(_) => {
                    record.rest_until = None;
                    record.failures = 0;
                }
                None => {}
            }
            ready.push((record.ms + f64::from(record.failures) * 3000., i, service));
        }
        if ready.is_empty() {
            return services.to_vec();
        }
        ready.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        ready.into_iter().map(|(_, _, service)| service).collect()
    }

    fn success(&mut self, service: Service, took: Duration) {
        let record = self.record(service);
        record.ms = record.ms * 0.7 + took.as_secs_f64() * 1000. * 0.3;
        record.failures = 0;
        record.rest_until = None;
    }

    fn failure(&mut self, service: Service, failure: &Failure, now: Instant) {
        let record = self.record(service);
        record.failures += 1;
        let rest = match failure {
            Failure::Limited(_) => Some(Duration::from_secs(5 * 60)),
            Failure::Denied(_) => Some(Duration::from_secs(6 * 60 * 60)),
            Failure::Other(_) if record.failures >= 3 => Some(Duration::from_secs(60)),
            Failure::Other(_) => None,
        };
        if let Some(rest) = rest {
            record.rest_until = Some(now + rest);
        }
    }
}

/// 整个进程共用的各家表现
pub static STATS: Mutex<Option<Stats>> = Mutex::new(None);

/// 一段的进度
struct Piece {
    /// 已经交给过几家（`order` 的前几个）
    tried: usize,
    /// 正在翻的：哪家、什么时候开始的
    running: Vec<(Service, Instant)>,
    last_launch: Instant,
    result: Option<PieceResult>,
    reasons: Vec<String>,
}

impl Piece {
    /// 交给下一家，返回是哪家；没有下一家了、手上也没在翻的，这段就失败了
    fn next(&mut self, order: &[Service], now: Instant) -> Option<Service> {
        match order.get(self.tried) {
            Some(&service) => {
                self.tried += 1;
                self.running.push((service, now));
                self.last_launch = now;
                Some(service)
            }
            None => {
                if self.running.is_empty() && self.result.is_none() {
                    self.result = Some(Err(self.reasons.join("；")));
                }
                None
            }
        }
    }
}

fn note_failure(stats: &Mutex<Option<Stats>>, service: Service, failure: &Failure) {
    if let Ok(mut guard) = stats.lock() {
        guard
            .get_or_insert_with(Stats::default)
            .failure(service, failure, Instant::now());
    }
}

/// 把 `pieces` 分给 `services` 翻。`valid(原文, 译文)` 检查译文能不能用；`progress(翻好几段, 一共几段)`
#[allow(clippy::too_many_arguments)]
pub fn run(
    pieces: Vec<String>,
    target: Target,
    services: &[Service],
    translator: Arc<dyn Translator>,
    stats: &Mutex<Option<Stats>>,
    valid: &dyn Fn(&str, &str) -> Result<(), String>,
    progress: &dyn Fn(usize, usize),
    timing: &Timing,
) -> Report {
    let total = pieces.len();
    let order = {
        let mut guard = stats.lock().unwrap_or_else(|e| e.into_inner());
        guard
            .get_or_insert_with(Stats::default)
            .order(services, Instant::now())
    };
    if order.is_empty() {
        return Report {
            results: vec![Err("没有可用的翻译服务".into()); total],
            hedges: 0,
        };
    }
    let pieces: Vec<Arc<str>> = pieces.into_iter().map(Arc::from).collect();
    let (sender, replies) = mpsc::channel::<(usize, Service, Duration, Result<String, Failure>)>();
    let launch = |index: usize, service: Service| {
        let (translator, text, sender) = (translator.clone(), pieces[index].clone(), sender.clone());
        thread::spawn(move || {
            let started = Instant::now();
            let reply = translator.translate(service, &text, target);
            let _ = sender.send((index, service, started.elapsed(), reply));
        });
    };
    let hedge = |index: usize| timing.hedge(pieces[index].chars().count());
    let now = Instant::now();
    let mut state: Vec<Piece> = (0..total)
        .map(|_| Piece {
            tried: 0,
            running: Vec::new(),
            last_launch: now,
            result: None,
            reasons: Vec::new(),
        })
        .collect();
    let mut waiting: VecDeque<usize> = (0..total).collect();
    let mut active: Vec<usize> = Vec::new();
    let (mut done, mut hedges) = (0, 0);
    loop {
        let now = Instant::now();
        while active.len() < PIECES_IN_FLIGHT
            && let Some(index) = waiting.pop_front()
        {
            active.push(index);
            if let Some(service) = state[index].next(&order, now) {
                launch(index, service);
            }
        }
        // 超时的算失败；手上没有在翻的换下一家；到了补位时间的再问下一家
        for &index in &active {
            let piece = &mut state[index];
            let limit = timing.attempt_limit;
            let (late, still): (Vec<_>, Vec<_>) = piece
                .running
                .iter()
                .partition(|(_, at)| now.duration_since(*at) >= limit);
            piece.running = still;
            for (service, _) in late {
                piece.reasons.push(format!("{}超时", service.name()));
                note_failure(stats, service, &Failure::Other("超时".into()));
            }
            if piece.result.is_some() {
                continue;
            }
            let due = now >= piece.last_launch + hedge(index);
            if piece.running.is_empty() || due {
                let hedging = !piece.running.is_empty();
                if let Some(service) = piece.next(&order, now) {
                    hedges += usize::from(hedging);
                    launch(index, service);
                }
            }
        }
        active.retain(|&index| state[index].result.is_none());
        if active.is_empty() && waiting.is_empty() {
            break;
        }
        if active.len() < PIECES_IN_FLIGHT && !waiting.is_empty() {
            continue;
        }
        let mut wake = now + Duration::from_secs(1);
        for &index in &active {
            let piece = &state[index];
            if piece.tried < order.len() {
                wake = wake.min(piece.last_launch + hedge(index));
            }
            for &(_, at) in &piece.running {
                wake = wake.min(at + timing.attempt_limit);
            }
        }
        let Ok((index, service, took, reply)) =
            replies.recv_timeout(wake.saturating_duration_since(Instant::now()))
        else {
            continue;
        };
        let reply = reply.and_then(|text| {
            valid(&pieces[index], &text)
                .map(|()| text)
                .map_err(Failure::Other)
        });
        match &reply {
            Ok(_) => {
                if let Ok(mut guard) = stats.lock() {
                    guard
                        .get_or_insert_with(Stats::default)
                        .success(service, took);
                }
            }
            Err(failure) => note_failure(stats, service, failure),
        }
        let piece = &mut state[index];
        piece.running.retain(|(running, _)| *running != service);
        if piece.result.is_some() {
            continue;
        }
        match reply {
            Ok(text) => {
                piece.result = Some(Ok((text, service)));
                done += 1;
                progress(done, total);
            }
            Err(failure) => {
                piece
                    .reasons
                    .push(format!("{}：{}", service.name(), failure.reason()));
                // 手上没有别家在翻了：马上换下一家，不用等到补位的时间
                if piece.running.is_empty()
                    && let Some(service) = piece.next(&order, Instant::now())
                {
                    launch(index, service);
                }
            }
        }
    }
    Report {
        results: state
            .into_iter()
            .map(|piece| piece.result.unwrap_or_else(|| Err("没翻".into())))
            .collect(),
        hedges,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 假的服务：每家按（等多久, 回什么）回答，记下谁被问了哪段
    struct Fake {
        answers: HashMap<Service, (Duration, Result<String, Failure>)>,
        asked: Mutex<Vec<(Service, String)>>,
    }

    impl Translator for Fake {
        fn translate(&self, service: Service, text: &str, _: Target) -> Result<String, Failure> {
            self.asked
                .lock()
                .unwrap()
                .push((service, text.to_owned()));
            let (wait, answer) = self.answers[&service].clone();
            thread::sleep(wait);
            answer.map(|reply| format!("{reply}:{text}"))
        }
    }

    const FAST: Timing = Timing {
        hedge_base: Duration::from_millis(200),
        hedge_per_300: Duration::from_millis(100),
        hedge_max: Duration::from_millis(600),
        attempt_limit: Duration::from_millis(1500),
    };

    fn fake(answers: &[(Service, u64, Result<&str, Failure>)]) -> Arc<Fake> {
        Arc::new(Fake {
            answers: answers
                .iter()
                .map(|(service, ms, reply)| {
                    (
                        *service,
                        (
                            Duration::from_millis(*ms),
                            reply.clone().map(str::to_owned),
                        ),
                    )
                })
                .collect(),
            asked: Mutex::new(Vec::new()),
        })
    }

    fn go(fake: Arc<Fake>, pieces: &[&str], services: &[Service]) -> (Report, Duration) {
        let stats = Mutex::new(Some(Stats::default()));
        let started = Instant::now();
        let report = run(
            pieces.iter().map(|p| p.to_string()).collect(),
            Target::English,
            services,
            fake,
            &stats,
            &|_, reply| {
                if reply.starts_with("bad") {
                    Err("译文不能用".into())
                } else {
                    Ok(())
                }
            },
            &|_, _| {},
            &FAST,
        );
        (report, started.elapsed())
    }

    const VOLC: Service = Service::Volcengine;
    const MS: Service = Service::Microsoft;
    const BILI: Service = Service::Bilibili;

    #[test]
    fn every_piece_goes_to_the_first_service_when_it_answers_in_time() {
        let f = fake(&[(VOLC, 20, Ok("v")), (MS, 20, Ok("m"))]);
        let (report, _) = go(f.clone(), &["一", "二", "三"], &[VOLC, MS]);
        let texts: Vec<_> = report
            .results
            .iter()
            .map(|r| r.as_ref().unwrap().0.clone())
            .collect();
        assert_eq!(texts, ["v:一", "v:二", "v:三"]);
        assert_eq!(report.hedges, 0);
        assert!(f.asked.lock().unwrap().iter().all(|(s, _)| *s == VOLC));
    }

    // 主人 2026-11-24：不能一家 5 秒不回整篇就算失败；每段各自计时，慢了让下一家补
    #[test]
    fn a_slow_piece_is_also_given_to_the_next_service_and_the_first_answer_wins() {
        let f = fake(&[(VOLC, 1_000, Ok("v")), (MS, 30, Ok("m"))]);
        let (report, took) = go(f.clone(), &["一"], &[VOLC, MS]);
        assert_eq!(report.results[0].as_ref().unwrap(), &("m:一".to_owned(), MS));
        assert_eq!(report.hedges, 1);
        assert!(took < Duration::from_millis(800), "{took:?}");
    }

    #[test]
    fn a_failure_or_an_unusable_reply_moves_on_at_once() {
        let f = fake(&[
            (VOLC, 10, Err(Failure::Other("出错".into()))),
            (MS, 10, Ok("bad")),
            (BILI, 10, Ok("b")),
        ]);
        let (report, took) = go(f, &["一"], &[VOLC, MS, BILI]);
        assert_eq!(report.results[0].as_ref().unwrap().1, BILI);
        assert!(took < Duration::from_millis(150), "{took:?}");
    }

    #[test]
    fn a_piece_fails_only_when_every_service_failed() {
        let f = fake(&[
            (VOLC, 10, Err(Failure::Other("连不上".into()))),
            (MS, 10, Err(Failure::Limited("限流".into()))),
        ]);
        let (report, _) = go(f, &["一"], &[VOLC, MS]);
        let reason = report.results[0].as_ref().unwrap_err();
        assert!(reason.contains("火山") && reason.contains("微软"), "{reason}");
    }

    #[test]
    fn a_service_that_never_answers_counts_as_failed_after_the_limit() {
        let f = fake(&[(VOLC, 5_000, Ok("v"))]);
        let (report, took) = go(f, &["一"], &[VOLC]);
        assert!(report.results[0].as_ref().unwrap_err().contains("超时"));
        assert!(took < Duration::from_millis(2_500), "{took:?}");
    }

    // 1 万字切成二三十段：同时翻几段，总时间不是一段一段加起来
    #[test]
    fn many_pieces_are_translated_a_few_at_a_time() {
        let f = fake(&[(VOLC, 100, Ok("v"))]);
        let pieces: Vec<String> = (0..24).map(|i| format!("第{i}段")).collect();
        let refs: Vec<&str> = pieces.iter().map(String::as_str).collect();
        let (report, took) = go(f, &refs, &[VOLC]);
        assert!(report.results.iter().all(Result::is_ok));
        assert!(took < Duration::from_millis(1_000), "{took:?}");
    }

    #[test]
    fn a_rate_limited_service_rests_and_the_fastest_goes_first() {
        let mut stats = Stats::default();
        let now = Instant::now();
        assert_eq!(stats.order(&[VOLC, MS, BILI], now), [VOLC, MS, BILI]);
        stats.failure(VOLC, &Failure::Limited("限流".into()), now);
        assert_eq!(stats.order(&[VOLC, MS, BILI], now), [MS, BILI]);
        assert_eq!(
            stats.order(&[VOLC, MS, BILI], now + Duration::from_secs(301)),
            [VOLC, MS, BILI]
        );
        // 微软最近一直很快，排到前面
        for _ in 0..5 {
            stats.success(MS, Duration::from_millis(50));
        }
        assert_eq!(stats.order(&[VOLC, MS], now)[0], MS);
        // 都在歇：全拿来试
        stats.failure(MS, &Failure::Denied("Key 不对".into()), now);
        stats.failure(VOLC, &Failure::Limited("限流".into()), now);
        assert_eq!(stats.order(&[VOLC, MS], now).len(), 2);
    }
}
