//! 两家云端识别抢着连：主通道先连，`patience` 内没连好就同时连备用，谁先连好用谁；
//! 没用上的那家连好以后自己断开（接收端已经丢了，连好的会话跟着被丢掉）。
//! 声音只发给用上的那家。主通道一开始就报错的，直接只连备用。
use crate::Result;
use std::{
    sync::mpsc::{Receiver, TryRecvError},
    thread,
    time::{Duration, Instant},
};

/// 一家正在连的结果：连好了（Some）、被取消（None）或者出错
pub type Pending<T> = Receiver<Result<Option<T>>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Primary,
    Backup,
}

/// 用上的那家；`Backup` 时带上主通道没用上的原因（出错原文，或者「太慢」）
pub struct Won<T> {
    pub side: Side,
    pub turn: T,
    pub primary_miss: Option<String>,
}

/// 两家都不行返回主通道的错（主通道没错时是备用的错）；取消返回 Ok(None)
pub fn first_ready<T>(
    primary: Pending<T>,
    start_backup: impl FnOnce() -> Pending<T>,
    patience: Duration,
    cancelled: &dyn Fn() -> bool,
) -> Result<Option<Won<T>>> {
    let deadline = Instant::now() + patience;
    let mut start_backup = Some(start_backup);
    let mut backup: Option<Pending<T>> = None;
    let mut primary_error: Option<String> = None;
    let mut primary_done = false;
    let mut backup_error = None;
    loop {
        if cancelled() {
            return Ok(None);
        }
        if !primary_done {
            match primary.try_recv() {
                Ok(Ok(Some(turn))) => {
                    return Ok(Some(Won {
                        side: Side::Primary,
                        turn,
                        primary_miss: None,
                    }));
                }
                Ok(Ok(None)) => return Ok(None),
                Ok(Err(error)) => {
                    primary_done = true;
                    primary_error = Some(error.to_string());
                }
                Err(TryRecvError::Disconnected) => {
                    primary_done = true;
                    primary_error = Some("主通道连接线程意外退出".into());
                }
                Err(TryRecvError::Empty) => {}
            }
        }
        if backup.is_none() && (primary_done || Instant::now() >= deadline) {
            backup = start_backup.take().map(|start| start());
        }
        if let Some(pending) = backup.as_ref() {
            match pending.try_recv() {
                Ok(Ok(Some(turn))) => {
                    return Ok(Some(Won {
                        side: Side::Backup,
                        turn,
                        primary_miss: Some(primary_error.unwrap_or_else(|| "太慢".into())),
                    }));
                }
                Ok(Ok(None)) => return Ok(None),
                Ok(Err(error)) => backup_error = Some(error),
                Err(TryRecvError::Disconnected) => {
                    backup_error = Some("备用通道连接线程意外退出".into());
                }
                Err(TryRecvError::Empty) => {}
            }
            if backup_error.is_some() {
                backup = None;
            }
        }
        if primary_done && backup_error.is_some() {
            return Err(match primary_error {
                Some(message) => message.into(),
                None => backup_error.take().expect("checked above"),
            });
        }
        thread::sleep(Duration::from_millis(5));
    }
}

/// 只等一家连好（另一家这一轮已经放弃了）；取消返回 Ok(None)
pub fn wait<T>(pending: Pending<T>, cancelled: &dyn Fn() -> bool) -> Result<Option<T>> {
    loop {
        if cancelled() {
            return Ok(None);
        }
        match pending.recv_timeout(Duration::from_millis(20)) {
            Ok(result) => return result,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                return Err("连接线程意外退出".into());
            }
        }
    }
}

/// 在后台线程里连一家，马上返回它的结果接收端
pub fn spawn<T: Send + 'static>(
    connect: impl FnOnce() -> Result<Option<T>> + Send + 'static,
) -> Pending<T> {
    let (sender, receiver) = std::sync::mpsc::channel();
    thread::spawn(move || {
        // 没人等了（另一家先连好）就丢掉，连好的会话随之断开
        let _ = sender.send(connect());
    });
    receiver
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    const PATIENCE: Duration = Duration::from_millis(150);

    fn after(ms: u64, result: Result<Option<&'static str>>) -> Pending<&'static str> {
        spawn(move || {
            thread::sleep(Duration::from_millis(ms));
            result
        })
    }

    fn never() -> bool {
        false
    }

    #[test]
    fn a_quick_primary_is_used_and_the_backup_is_never_started() {
        let started = Arc::new(AtomicBool::new(false));
        let flag = started.clone();
        let won = first_ready(
            after(10, Ok(Some("豆包"))),
            move || {
                flag.store(true, Ordering::SeqCst);
                after(0, Ok(Some("微信")))
            },
            PATIENCE,
            &never,
        )
        .unwrap()
        .unwrap();
        assert_eq!((won.side, won.turn), (Side::Primary, "豆包"));
        assert!(!started.load(Ordering::SeqCst));
    }

    #[test]
    fn a_slow_primary_loses_to_a_ready_backup() {
        let won = first_ready(
            after(2_000, Ok(Some("豆包"))),
            || after(10, Ok(Some("微信"))),
            PATIENCE,
            &never,
        )
        .unwrap()
        .unwrap();
        assert_eq!((won.side, won.turn), (Side::Backup, "微信"));
        assert_eq!(won.primary_miss.as_deref(), Some("太慢"));
    }

    #[test]
    fn a_failing_primary_goes_straight_to_the_backup_with_its_reason() {
        let begun = Instant::now();
        let won = first_ready(
            after(5, Err("connection timed out".into())),
            || after(10, Ok(Some("微信"))),
            Duration::from_secs(5),
            &never,
        )
        .unwrap()
        .unwrap();
        assert_eq!(won.side, Side::Backup);
        assert_eq!(won.primary_miss.as_deref(), Some("connection timed out"));
        assert!(begun.elapsed() < Duration::from_secs(1), "不等满耐心时间");
    }

    #[test]
    fn a_slow_primary_still_wins_if_it_beats_the_backup() {
        let won = first_ready(
            after(200, Ok(Some("豆包"))),
            || after(1_000, Ok(Some("微信"))),
            PATIENCE,
            &never,
        )
        .unwrap()
        .unwrap();
        assert_eq!(won.side, Side::Primary);
    }

    #[test]
    fn when_both_fail_the_primary_error_is_reported() {
        let error = first_ready(
            after(5, Err("豆包坏了".into())),
            || after(5, Err("微信也坏了".into())),
            PATIENCE,
            &never,
        )
        .err()
        .unwrap();
        assert_eq!(error.to_string(), "豆包坏了");
    }

    #[test]
    fn waiting_for_one_channel_returns_its_result() {
        assert_eq!(
            wait(after(20, Ok(Some("微信"))), &never).unwrap(),
            Some("微信")
        );
        assert!(wait(after(5, Err("坏了".into())), &never).is_err());
        assert_eq!(
            wait(after(1_000, Ok(Some("微信"))), &|| true).unwrap(),
            None
        );
    }

    #[test]
    fn cancelling_returns_nothing() {
        let stop = AtomicBool::new(false);
        let primary = after(2_000, Ok(Some("豆包")));
        stop.store(true, Ordering::SeqCst);
        assert!(
            first_ready(primary, || after(0, Ok(Some("微信"))), PATIENCE, &|| stop
                .load(Ordering::SeqCst))
            .unwrap()
            .is_none()
        );
    }
}
