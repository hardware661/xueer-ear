//! 面板此刻该显示什么（纯逻辑，可在 WSL 测）。
//! 出错提示要留几秒给主人看：这期间别的线程说「收起」不算数，到点自己收；新一轮听写可以盖掉它。
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Look {
    Hidden,
    /// 麦克风：在听、还没听到说话
    Listening,
    /// 听到在说话：中间变成随音量起伏的波浪
    Speaking,
    /// 正在连接云端
    Connecting,
    /// 灰色麦克风（只在离屏预览里用）
    Idle,
    /// 出错提示：卡片变宽，写一句原因
    Notice,
}

/// 面板上点到了哪（主人 2026-11-19：左边设置、中间麦克风、右边翻译小窗）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Click {
    Settings,
    Mic,
    Translate,
}

/// 面板宽度（96 DPI 像素），三个按钮横排
pub const PANEL_WIDTH: f32 = 176.;
/// 三个按钮的圆心 x（中间是麦克风）
pub const SETTINGS_X: f32 = 36.;
pub const TRANSLATE_X: f32 = PANEL_WIDTH - 36.;

/// 点在 (x, y)（面板内的逻辑坐标）算点了哪个按钮；两边按钮之外的地方都算麦克风
pub fn hit(x: f32, y: f32, center_y: f32) -> Click {
    let near = |cx: f32| (x - cx).powi(2) + (y - center_y).powi(2) <= 20_f32.powi(2);
    if near(SETTINGS_X) {
        Click::Settings
    } else if near(TRANSLATE_X) {
        Click::Translate
    } else {
        Click::Mic
    }
}

/// 出错提示留多久
pub const NOTICE_HOLD: Duration = Duration::from_secs(4);

pub struct Board {
    look: Look,
    text: String,
    until: Option<Instant>,
    after_notice: Look,
}

impl Default for Board {
    fn default() -> Self {
        Self {
            look: Look::Hidden,
            text: String::new(),
            until: None,
            after_notice: Look::Hidden,
        }
    }
}

impl Board {
    pub fn look(&self) -> Look {
        self.look
    }

    /// 提示的文字（不在提示时是空的）
    pub fn text(&self) -> &str {
        &self.text
    }

    /// 换外观，返回有没有变。提示还没到点时「收起」不生效。
    pub fn set(&mut self, look: Look, now: Instant) -> bool {
        if look == self.after_notice && self.until.is_some_and(|until| now < until) {
            return false;
        }
        let changed = self.look != look;
        self.look = look;
        self.text.clear();
        self.until = None;
        changed
    }

    /// 显示一句提示，留 `NOTICE_HOLD` 这么久
    pub fn notice(&mut self, text: &str, now: Instant) {
        self.show_notice(text, now, Look::Hidden);
    }

    /// 翻译失败并未停止听写，提示到期后仍显示麦克风。
    pub fn listening_notice(&mut self, text: &str, now: Instant) {
        self.show_notice(text, now, Look::Listening);
    }

    fn show_notice(&mut self, text: &str, now: Instant, after_notice: Look) {
        self.look = Look::Notice;
        self.text = text.to_string();
        self.until = Some(now + NOTICE_HOLD);
        self.after_notice = after_notice;
    }

    /// 定时检查：提示到点了就收起。返回 true 表示刚收起。
    pub fn expire(&mut self, now: Instant) -> bool {
        if self.look == Look::Notice && self.until.is_none_or(|until| now >= until) {
            self.dismiss();
            return true;
        }
        false
    }

    /// 主人点了提示：马上收起。返回 true 表示当时确实在提示。
    pub fn dismiss(&mut self) -> bool {
        if self.look != Look::Notice {
            return false;
        }
        self.look = self.after_notice;
        self.text.clear();
        self.until = None;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notice_survives_hide_requests_until_it_expires() {
        let start = Instant::now();
        let mut board = Board::default();
        board.set(Look::Listening, start);
        board.notice("网络不通，这句没识别出来", start);
        assert_eq!(board.look(), Look::Notice);
        assert_eq!(board.text(), "网络不通，这句没识别出来");
        // 听写线程结束后，轮询线程会一直要求收起
        assert!(!board.set(Look::Hidden, start + Duration::from_secs(1)));
        assert_eq!(board.look(), Look::Notice);
        assert!(!board.expire(start + Duration::from_secs(3)));
        assert!(board.expire(start + NOTICE_HOLD));
        assert_eq!(board.look(), Look::Hidden);
        assert_eq!(board.text(), "");
        assert!(!board.expire(start + NOTICE_HOLD));
    }

    #[test]
    fn translation_notice_returns_to_listening_but_does_not_block_stop() {
        let start = Instant::now();
        let mut board = Board::default();
        board.listening_notice("翻译失败", start);
        assert!(!board.set(Look::Listening, start + Duration::from_secs(1)));
        assert_eq!(board.look(), Look::Notice);
        assert!(board.expire(start + NOTICE_HOLD));
        assert_eq!(board.look(), Look::Listening);

        board.listening_notice("翻译失败", start);
        assert!(board.set(Look::Hidden, start + Duration::from_secs(1)));
        assert_eq!(board.look(), Look::Hidden);
        assert!(!board.expire(start + NOTICE_HOLD));
    }

    #[test]
    fn a_new_dictation_replaces_the_notice_at_once() {
        let start = Instant::now();
        let mut board = Board::default();
        board.notice("这次出错了，请再说一遍", start);
        assert!(board.set(Look::Connecting, start + Duration::from_millis(500)));
        assert_eq!(board.look(), Look::Connecting);
        assert_eq!(board.text(), "");
        // 之后的收起照常生效
        assert!(board.set(Look::Hidden, start + Duration::from_secs(1)));
    }

    #[test]
    fn click_dismisses_only_a_notice() {
        let start = Instant::now();
        let mut board = Board::default();
        board.set(Look::Listening, start);
        assert!(!board.dismiss());
        assert_eq!(board.look(), Look::Listening);
        board.notice("麦克风打不开，请检查设备", start);
        assert!(board.dismiss());
        assert_eq!(board.look(), Look::Hidden);
    }

    #[test]
    fn side_buttons_and_mic_are_hit_where_drawn() {
        let cy = 59.;
        assert_eq!(hit(SETTINGS_X, cy, cy), Click::Settings);
        assert_eq!(hit(TRANSLATE_X + 10., cy + 5., cy), Click::Translate);
        assert_eq!(hit(PANEL_WIDTH / 2., cy, cy), Click::Mic);
        // 耳朵、空白处照旧算麦克风
        assert_eq!(hit(10., 5., cy), Click::Mic);
    }

    #[test]
    fn set_reports_changes_only() {
        let start = Instant::now();
        let mut board = Board::default();
        assert!(!board.set(Look::Hidden, start));
        assert!(board.set(Look::Listening, start));
        assert!(!board.set(Look::Listening, start));
        assert!(board.set(Look::Speaking, start));
    }
}
