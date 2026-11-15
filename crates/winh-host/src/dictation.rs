//! 雪耳面板的听写状态机（纯逻辑，可在 WSL 测）。交互仿 Win+H：
//! 点一下热键打开面板并开始听；停顿收句后麦克风变灰、面板留着；再点热键或点麦克风接着听；
//! 按住热键说、松开收句；点 ×、Esc 或灰着放一会儿就收起。
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Panel {
    Hidden,
    /// 正在听（蓝色麦克风）
    Listening,
    /// 面板开着、没在听（灰色麦克风）
    Idle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Input {
    /// 热键按下（只在从松开到按下的那一刻发一次）
    KeyDown,
    KeyUp,
    /// 点了面板上的麦克风
    MicClick,
    /// 点了 × 或按了 Esc
    Close,
    /// 识别那边这一轮结束了（停顿收句、出错或被取消）
    SessionEnded,
    /// 按热键时还按着别的修饰键（组合键）
    Chord,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    Show,
    Hide,
    /// 开始一轮听写
    Start,
    /// 停止采音，让这一轮正常收尾出定稿
    Finish,
}

pub struct Dictation {
    hold_threshold: Duration,
    idle_timeout: Duration,
    panel: Panel,
    /// 热键按下的时刻；按着时为 Some。只有这次按下开启了听写，松开才可能收句
    pressed: Option<(Instant, bool)>,
    idle_since: Option<Instant>,
}

impl Dictation {
    pub fn new(hold_threshold: Duration, idle_timeout: Duration) -> Self {
        Self {
            hold_threshold,
            idle_timeout,
            panel: Panel::Hidden,
            pressed: None,
            idle_since: None,
        }
    }

    pub fn panel(&self) -> Panel {
        self.panel
    }

    pub fn input(&mut self, now: Instant, input: Input) -> Vec<Effect> {
        match (self.panel, input) {
            (_, Input::Chord) => vec![],
            (Panel::Hidden, Input::KeyDown) => {
                self.pressed = Some((now, true));
                self.listen();
                vec![Effect::Show, Effect::Start]
            }
            (Panel::Idle, Input::KeyDown) => {
                self.pressed = Some((now, true));
                self.listen();
                vec![Effect::Start]
            }
            (Panel::Listening, Input::KeyDown) => {
                // 正在听时点一下：收句（松开时不再重复处理）
                self.pressed = Some((now, false));
                self.go_idle(now);
                vec![Effect::Finish]
            }
            (_, Input::KeyUp) => {
                let held = self.pressed.take();
                match held {
                    Some((at, true))
                        if self.panel == Panel::Listening
                            && now.duration_since(at) >= self.hold_threshold =>
                    {
                        self.go_idle(now);
                        vec![Effect::Finish]
                    }
                    _ => vec![],
                }
            }
            (Panel::Idle, Input::MicClick) => {
                self.listen();
                vec![Effect::Start]
            }
            (Panel::Listening, Input::MicClick) => {
                self.go_idle(now);
                vec![Effect::Finish]
            }
            (Panel::Listening, Input::SessionEnded) => {
                self.go_idle(now);
                vec![]
            }
            (Panel::Listening, Input::Close) => {
                self.hide();
                vec![Effect::Finish, Effect::Hide]
            }
            (Panel::Idle, Input::Close) => {
                self.hide();
                vec![Effect::Hide]
            }
            (Panel::Hidden, _) | (Panel::Idle, Input::SessionEnded) => vec![],
        }
    }

    /// 定时调用：灰着超过 idle_timeout 就收起
    pub fn tick(&mut self, now: Instant) -> Vec<Effect> {
        match self.idle_since {
            Some(since)
                if self.panel == Panel::Idle && now.duration_since(since) > self.idle_timeout =>
            {
                self.hide();
                vec![Effect::Hide]
            }
            _ => vec![],
        }
    }

    fn listen(&mut self) {
        self.panel = Panel::Listening;
        self.idle_since = None;
    }

    fn go_idle(&mut self, now: Instant) {
        self.panel = Panel::Idle;
        self.idle_since = Some(now);
    }

    fn hide(&mut self) {
        self.panel = Panel::Hidden;
        self.idle_since = None;
        self.pressed = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Effect::*;
    use Input::*;

    const HOLD: Duration = Duration::from_millis(400);
    const IDLE: Duration = Duration::from_secs(10);

    struct Clock(Instant, Dictation);
    impl Clock {
        fn new() -> Self {
            Self(Instant::now(), Dictation::new(HOLD, IDLE))
        }
        fn at(&mut self, ms: u64, input: Input) -> Vec<Effect> {
            self.1.input(self.0 + Duration::from_millis(ms), input)
        }
        fn tick(&mut self, ms: u64) -> Vec<Effect> {
            self.1.tick(self.0 + Duration::from_millis(ms))
        }
    }

    #[test]
    fn tap_opens_panel_and_keeps_listening_after_release() {
        let mut c = Clock::new();
        assert_eq!(c.at(0, KeyDown), [Show, Start]);
        assert_eq!(c.at(150, KeyUp), []);
        assert_eq!(c.1.panel(), Panel::Listening);
    }

    #[test]
    fn hold_then_release_finishes_the_sentence() {
        let mut c = Clock::new();
        assert_eq!(c.at(0, KeyDown), [Show, Start]);
        assert_eq!(c.at(2000, KeyUp), [Finish]);
    }

    #[test]
    fn tap_while_listening_finishes() {
        let mut c = Clock::new();
        c.at(0, KeyDown);
        c.at(100, KeyUp);
        assert_eq!(c.at(5000, KeyDown), [Finish]);
        assert_eq!(c.at(5100, KeyUp), []);
    }

    #[test]
    fn silence_end_turns_grey_and_tap_or_mic_resumes() {
        let mut c = Clock::new();
        c.at(0, KeyDown);
        c.at(100, KeyUp);
        assert_eq!(c.at(8000, SessionEnded), []);
        assert_eq!(c.1.panel(), Panel::Idle);
        assert_eq!(c.at(9000, MicClick), [Start]);
        assert_eq!(c.1.panel(), Panel::Listening);
        c.at(12000, SessionEnded);
        assert_eq!(c.at(13000, KeyDown), [Start]);
    }

    #[test]
    fn mic_click_while_listening_finishes() {
        let mut c = Clock::new();
        c.at(0, KeyDown);
        c.at(100, KeyUp);
        assert_eq!(c.at(3000, MicClick), [Finish]);
    }

    #[test]
    fn idle_panel_hides_after_timeout_but_not_while_listening() {
        let mut c = Clock::new();
        c.at(0, KeyDown);
        c.at(100, KeyUp);
        assert_eq!(c.tick(60_000), []);
        c.at(61_000, SessionEnded);
        assert_eq!(c.tick(70_000), []);
        assert_eq!(c.tick(71_001), [Hide]);
        assert_eq!(c.1.panel(), Panel::Hidden);
    }

    #[test]
    fn close_finishes_and_hides() {
        let mut c = Clock::new();
        c.at(0, KeyDown);
        c.at(100, KeyUp);
        assert_eq!(c.at(2000, Close), [Finish, Hide]);
        let mut c = Clock::new();
        c.at(0, KeyDown);
        c.at(100, KeyUp);
        c.at(500, SessionEnded);
        assert_eq!(c.at(2000, Close), [Hide]);
    }

    #[test]
    fn chord_is_ignored_and_does_not_open() {
        let mut c = Clock::new();
        assert_eq!(c.at(0, Chord), []);
        assert_eq!(c.1.panel(), Panel::Hidden);
    }

    #[test]
    fn session_end_while_key_held_does_not_restart() {
        let mut c = Clock::new();
        c.at(0, KeyDown);
        assert_eq!(c.at(500, SessionEnded), []);
        assert_eq!(c.at(900, KeyUp), []);
        assert_eq!(c.1.panel(), Panel::Idle);
    }

    #[test]
    fn hidden_panel_ignores_stray_events() {
        let mut c = Clock::new();
        assert_eq!(c.at(0, KeyUp), []);
        assert_eq!(c.at(0, SessionEnded), []);
        assert_eq!(c.at(0, MicClick), []);
        assert_eq!(c.at(0, Close), []);
    }
}
