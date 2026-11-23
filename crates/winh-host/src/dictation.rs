//! 雪耳面板的听写状态机（纯逻辑，可在 WSL 测）。
//! 主人 2026-11-15 定：点一下热键开始；再点一下（或点麦克风、Esc）就停止并收起面板；
//! 一阵子没识别出新字，听写自己结束，面板也收起。没有「灰着等着」的状态。
//! 主人 2026-11-24 砍掉了「按住说话、松开结束」：按住右 Alt 时只弹出面板、不出字，
//! 编辑器的菜单栏还被 Alt 切来切去。现在按多久都只算点一下，松开什么都不做。
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Panel {
    Hidden,
    Listening,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Input {
    /// 热键按下（只在从松开到按下的那一刻发一次）
    KeyDown,
    /// 热键松开（什么都不做；按住说话已经去掉）
    KeyUp,
    /// 点了面板上的麦克风
    MicClick,
    /// 按了 Esc
    Close,
    /// 听写那边自己结束了（太久没识别出新字、出错或到最长时间）
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
    /// 停止采音，让最后一句正常收尾上屏
    Finish,
}

pub struct Dictation {
    panel: Panel,
}

impl Dictation {
    pub fn new() -> Self {
        Self {
            panel: Panel::Hidden,
        }
    }

    pub fn panel(&self) -> Panel {
        self.panel
    }

    pub fn input(&mut self, _now: Instant, input: Input) -> Vec<Effect> {
        match (self.panel, input) {
            (_, Input::Chord | Input::KeyUp) => vec![],
            (Panel::Hidden, Input::KeyDown) => {
                self.panel = Panel::Listening;
                vec![Effect::Show, Effect::Start]
            }
            (Panel::Listening, Input::KeyDown) => self.stop(),
            (Panel::Listening, Input::MicClick | Input::Close) => self.stop(),
            (Panel::Listening, Input::SessionEnded) => {
                self.panel = Panel::Hidden;
                vec![Effect::Hide]
            }
            (Panel::Hidden, _) => vec![],
        }
    }

    fn stop(&mut self) -> Vec<Effect> {
        self.panel = Panel::Hidden;
        vec![Effect::Finish, Effect::Hide]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Effect::*;
    use Input::*;

    use std::time::Duration;

    struct Clock(Instant, Dictation);
    impl Clock {
        fn new() -> Self {
            Self(Instant::now(), Dictation::new())
        }
        fn at(&mut self, ms: u64, input: Input) -> Vec<Effect> {
            self.1.input(self.0 + Duration::from_millis(ms), input)
        }
    }

    #[test]
    fn tap_starts_and_keeps_listening_after_release() {
        let mut c = Clock::new();
        assert_eq!(c.at(0, KeyDown), [Show, Start]);
        assert_eq!(c.at(150, KeyUp), []);
        assert_eq!(c.1.panel(), Panel::Listening);
    }

    #[test]
    fn second_tap_stops_and_hides() {
        let mut c = Clock::new();
        c.at(0, KeyDown);
        c.at(100, KeyUp);
        assert_eq!(c.at(5000, KeyDown), [Finish, Hide]);
        assert_eq!(c.1.panel(), Panel::Hidden);
        assert_eq!(c.at(5100, KeyUp), []);
    }

    // 主人 2026-11-24：按住说话去掉了，按多久松开都只是点一下
    #[test]
    fn holding_the_key_then_releasing_keeps_listening() {
        let mut c = Clock::new();
        c.at(0, KeyDown);
        assert_eq!(c.at(2000, KeyUp), []);
        assert_eq!(c.1.panel(), Panel::Listening);
        assert_eq!(c.at(5000, KeyDown), [Finish, Hide]);
    }

    #[test]
    fn mic_click_and_close_stop_and_hide() {
        let mut c = Clock::new();
        c.at(0, KeyDown);
        c.at(100, KeyUp);
        assert_eq!(c.at(3000, MicClick), [Finish, Hide]);
        let mut c = Clock::new();
        c.at(0, KeyDown);
        c.at(100, KeyUp);
        assert_eq!(c.at(3000, Close), [Finish, Hide]);
    }

    #[test]
    fn session_ending_by_itself_hides() {
        let mut c = Clock::new();
        c.at(0, KeyDown);
        c.at(100, KeyUp);
        assert_eq!(c.at(12000, SessionEnded), [Hide]);
        assert_eq!(c.1.panel(), Panel::Hidden);
        // 再点一下重新开始
        assert_eq!(c.at(13000, KeyDown), [Show, Start]);
    }

    #[test]
    fn session_end_while_key_held_hides_and_release_is_ignored() {
        let mut c = Clock::new();
        c.at(0, KeyDown);
        assert_eq!(c.at(500, SessionEnded), [Hide]);
        assert_eq!(c.at(900, KeyUp), []);
    }

    #[test]
    fn chord_is_ignored() {
        let mut c = Clock::new();
        assert_eq!(c.at(0, Chord), []);
        assert_eq!(c.1.panel(), Panel::Hidden);
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
