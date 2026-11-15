//! 快捷键模式的按键状态机（纯逻辑，可在 WSL 测）。
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// 开始录音
    Start,
    /// 松开：停止录音，让这句正常收尾上屏
    Finish,
    /// 取消：丢掉这次
    Cancel,
}

pub struct HotkeyState {
    min_hold: Duration,
    phase: Phase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// 热键没按着
    Idle,
    /// 正在录音，记下开始时间
    Recording(Instant),
    /// 热键还按着但这次不算（组合键、被取消、上一次没收尾），等松开
    Ignored,
}

impl HotkeyState {
    pub fn new(min_hold: Duration) -> Self {
        Self {
            min_hold,
            phase: Phase::Idle,
        }
    }

    /// 每次轮询调用：`held` 热键是否按着，`other` 是否有别的键按着，`busy` 上一次是否还没收尾。
    /// 按得太短（误触）算取消；按住时又按了别的键，当作组合键，取消并等松开。
    pub fn poll(&mut self, now: Instant, held: bool, other: bool, busy: bool) -> Option<Action> {
        match (self.phase, held) {
            (Phase::Idle, false) | (Phase::Ignored, true) => None,
            (Phase::Ignored, false) => {
                self.phase = Phase::Idle;
                None
            }
            (Phase::Idle, true) => {
                if other || busy {
                    self.phase = Phase::Ignored;
                    None
                } else {
                    self.phase = Phase::Recording(now);
                    Some(Action::Start)
                }
            }
            (Phase::Recording(_), true) if other => {
                self.phase = Phase::Ignored;
                Some(Action::Cancel)
            }
            (Phase::Recording(_), true) => None,
            (Phase::Recording(started), false) => {
                self.phase = Phase::Idle;
                Some(if now.duration_since(started) < self.min_hold {
                    Action::Cancel
                } else {
                    Action::Finish
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: Duration = Duration::from_millis(300);

    fn run(steps: &[(u64, bool, bool, bool)]) -> Vec<(u64, Action)> {
        let t0 = Instant::now();
        let mut s = HotkeyState::new(MIN);
        steps
            .iter()
            .filter_map(|&(ms, held, other, busy)| {
                s.poll(t0 + Duration::from_millis(ms), held, other, busy)
                    .map(|a| (ms, a))
            })
            .collect()
    }

    #[test]
    fn hold_then_release_finishes() {
        let got = run(&[
            (0, false, false, false),
            (10, true, false, false),
            (500, true, false, false),
            (900, false, false, false),
        ]);
        assert_eq!(got, [(10, Action::Start), (900, Action::Finish)]);
    }

    #[test]
    fn short_tap_cancels() {
        let got = run(&[(0, true, false, false), (100, false, false, false)]);
        assert_eq!(got, [(0, Action::Start), (100, Action::Cancel)]);
    }

    #[test]
    fn other_key_while_held_cancels_once_and_waits_for_release() {
        let got = run(&[
            (0, true, false, false),
            (400, true, true, false),
            (450, true, true, false),
            (500, true, false, false),
            (600, false, false, false),
            (700, true, false, false),
        ]);
        assert_eq!(
            got,
            [
                (0, Action::Start),
                (400, Action::Cancel),
                (700, Action::Start)
            ]
        );
    }

    #[test]
    fn combo_started_before_hotkey_never_starts() {
        // 先按着 Ctrl 再按右 Alt：是组合键，不是说话
        let got = run(&[
            (0, true, true, false),
            (500, true, false, false),
            (900, false, false, false),
        ]);
        assert!(got.is_empty(), "{got:?}");
    }

    #[test]
    fn busy_previous_session_blocks_new_start_until_release() {
        let got = run(&[
            (0, true, false, true),
            (500, true, false, false),
            (600, false, false, false),
            (700, true, false, false),
        ]);
        assert_eq!(got, [(700, Action::Start)]);
    }
}
