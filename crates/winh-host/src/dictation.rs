//! 雪耳面板的听写状态机（纯逻辑，可在 WSL 测）。
//! 主人 2026-11-15 定：点一下热键开始；再点一下（或点麦克风、Esc）就停止并收起面板；
//! 一阵子没识别出新字，听写自己结束，面板也收起。没有「灰着等着」的状态。
//! 主人 2026-11-24 砍掉了「按住说话、松开结束」，复测时又定：「按住」整个砍掉。
//! 按下热键什么都不做，松开那一下才算点了一下：按住期间不开始听，也就不会边按着 Alt 边打字，
//! Typora 这类程序的菜单栏不会被切出来，字也不用等。按热键时还按着别的修饰键算组合键，不算点。
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Panel {
    Hidden,
    Listening,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Input {
    /// 热键按下（只在从松开到按下的那一刻发一次）：什么都不做，记下这时面板开没开
    Press,
    /// 热键松开：这一下按完了，才算点了一下
    Release,
    /// 按着热键时还按着别的修饰键（组合键）：这一下不算点
    Chord,
    /// 点了面板上的麦克风
    MicClick,
    /// 按了 Esc
    Close,
    /// 听写那边自己结束了（太久没识别出新字、出错或到最长时间）
    SessionEnded,
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
    /// 热键按下时面板是什么样（还没松开；组合键、按下后面板变了就作废）
    pressed: Option<Panel>,
}

impl Dictation {
    pub fn new() -> Self {
        Self {
            panel: Panel::Hidden,
            pressed: None,
        }
    }

    pub fn panel(&self) -> Panel {
        self.panel
    }

    pub fn input(&mut self, _now: Instant, input: Input) -> Vec<Effect> {
        match (self.panel, input) {
            (panel, Input::Press) => {
                self.pressed = Some(panel);
                vec![]
            }
            (_, Input::Chord) => {
                self.pressed = None;
                vec![]
            }
            // 按着热键的时候面板变了（听写自己结束了、点了面板）：松开这一下不再算点
            (panel, Input::Release) if self.pressed.take() != Some(panel) => vec![],
            (Panel::Hidden, Input::Release) => {
                self.panel = Panel::Listening;
                vec![Effect::Show, Effect::Start]
            }
            (Panel::Listening, Input::Release) => self.stop(),
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
        /// 点一下：按下再松开
        fn tap(&mut self, ms: u64) -> Vec<Effect> {
            assert_eq!(self.at(ms, Press), []);
            self.at(ms + 120, Release)
        }
    }

    // 主人 2026-11-24 复测：按住右 Alt 时 Typora 里不出字、菜单栏来回切，「按住」整个砍掉：
    // 按下什么都不做，松开那一下才算点了一下
    #[test]
    fn pressing_does_nothing_and_releasing_is_the_tap() {
        let mut c = Clock::new();
        assert_eq!(c.at(0, Press), []);
        assert_eq!(c.1.panel(), Panel::Hidden);
        assert_eq!(c.at(120, Release), [Show, Start]);
        assert_eq!(c.1.panel(), Panel::Listening);
    }

    #[test]
    fn holding_the_key_long_is_still_one_tap_on_release() {
        let mut c = Clock::new();
        assert_eq!(c.at(0, Press), []);
        assert_eq!(c.at(3000, Release), [Show, Start]);
        assert_eq!(c.tap(8000), [Finish, Hide]);
    }

    #[test]
    fn second_tap_stops_and_hides() {
        let mut c = Clock::new();
        c.tap(0);
        assert_eq!(c.tap(5000), [Finish, Hide]);
        assert_eq!(c.1.panel(), Panel::Hidden);
    }

    #[test]
    fn another_modifier_during_the_hold_makes_it_a_chord_not_a_tap() {
        let mut c = Clock::new();
        c.at(0, Press);
        assert_eq!(c.at(300, Chord), []);
        assert_eq!(c.at(400, Release), []);
        assert_eq!(c.1.panel(), Panel::Hidden);
        // 听写中按了组合键也不算停
        c.tap(1000);
        c.at(2000, Press);
        c.at(2100, Chord);
        assert_eq!(c.at(2200, Release), []);
        assert_eq!(c.1.panel(), Panel::Listening);
    }

    #[test]
    fn a_session_that_ends_while_the_key_is_held_is_not_restarted_by_the_release() {
        let mut c = Clock::new();
        c.tap(0);
        c.at(5000, Press);
        assert_eq!(c.at(5300, SessionEnded), [Hide]);
        assert_eq!(c.at(5600, Release), []);
        assert_eq!(c.1.panel(), Panel::Hidden);
    }

    #[test]
    fn mic_click_and_close_stop_and_hide() {
        let mut c = Clock::new();
        c.tap(0);
        assert_eq!(c.at(3000, MicClick), [Finish, Hide]);
        let mut c = Clock::new();
        c.tap(0);
        assert_eq!(c.at(3000, Close), [Finish, Hide]);
    }

    #[test]
    fn session_ending_by_itself_hides() {
        let mut c = Clock::new();
        c.tap(0);
        assert_eq!(c.at(12000, SessionEnded), [Hide]);
        assert_eq!(c.1.panel(), Panel::Hidden);
        // 再点一下重新开始
        assert_eq!(c.tap(13000), [Show, Start]);
    }

    #[test]
    fn stray_events_are_ignored() {
        let mut c = Clock::new();
        // 服务启动时热键正按着：只看到松开，不算点
        assert_eq!(c.at(0, Release), []);
        assert_eq!(c.at(0, Chord), []);
        assert_eq!(c.at(0, SessionEnded), []);
        assert_eq!(c.at(0, MicClick), []);
        assert_eq!(c.at(0, Close), []);
        assert_eq!(c.1.panel(), Panel::Hidden);
    }
}
