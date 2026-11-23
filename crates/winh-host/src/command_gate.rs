//! 语音指令什么时候算数（主人 2026-11-24 定的规矩）：
//!
//! **先停够（默认 1 秒），单独说指令，再停一下（默认 0.6 秒）没接着说，才执行。**
//!
//! - 「停够」看本机断句报的「停了多久又开口」（`Event::Voice`）：这句话开口前停了多久。
//!   这一轮第一次开口前面没有话，算停够了。
//! - 「单独说」：这句（云端的一个分句）从头到尾只有指令的说法。说的过程中，只要到目前为止的字
//!   还是某个指令的开头（「删」「全部」「翻译成」），就先不打出来；一旦不是了，整句照常上屏。
//! - 「再停一下」：中间结果已经是完整的指令、停了 `settle` 没再变，就执行，不等云端定稿。
//! - 云端定稿把指令词吞了或改写了：豆包会把单独说的「撤回」吞成空的（2026-11-24，已经不是指令了，
//!   别的词也可能），微信会把「翻译成英文」那句自己翻成英文（2026-11-23）。这时按中间结果执行。
//! - 执行过指令后，这句后面要是接着说了别的话，那部分照常上屏。
use std::time::{Duration, Instant};

pub struct CommandGate {
    pause_ms: u32,
    settle: Duration,
    /// 最近一次开口前停了多少毫秒；这一轮还没开过口是 None
    last_pause: Option<u32>,
    segment: Segment,
}

#[derive(Default)]
struct Segment {
    /// 这句开口前停够了没有（这句第一次出字时定）
    standalone: Option<bool>,
    /// 还可能是指令：停够了，而且到目前为止的字都是某个指令的开头
    open: bool,
    /// 已经是完整指令的那版中间结果，以及从什么时候起没变
    pending: Option<(String, Instant)>,
    /// 这句里已经提前执行了的指令（说出来的那版中间结果）
    fired: Option<String>,
}

/// 一版中间结果怎么上屏
#[derive(Debug, PartialEq, Eq)]
pub enum Partial {
    /// 先不打（可能是指令）
    Hold,
    /// 打这些字（执行过指令的句子只打指令后面接着说的）
    Type(String),
}

/// 这句定稿了怎么办
#[derive(Debug, PartialEq, Eq)]
pub enum Finish {
    /// 当普通文字上屏（执行过指令的句子只剩指令后面的）
    Text(String),
    /// 执行这个指令（交回说出来的原话）
    Command(String),
}

impl CommandGate {
    pub fn new(pause: Duration, settle: Duration) -> Self {
        Self {
            pause_ms: u32::try_from(pause.as_millis()).unwrap_or(u32::MAX),
            settle,
            last_pause: None,
            segment: Segment::default(),
        }
    }

    /// 本机断句：停了 `pause_ms` 毫秒后又开口了
    pub fn voice(&mut self, pause_ms: u32) {
        self.last_pause = Some(pause_ms);
    }

    /// 这句开口前停够了没有：这一轮第一次开口，或者上次开口前停得够久
    fn paused_enough(&self) -> bool {
        self.last_pause.is_none_or(|pause| pause >= self.pause_ms)
    }

    /// 这句新的一版中间结果。`starts`：整句到目前为止是某个开着的指令的开头；`is_command`：整句就是指令
    pub fn partial(&mut self, text: &str, starts: bool, is_command: bool, now: Instant) -> Partial {
        if text.trim().is_empty() {
            // 云端把字吞了（变成空的）：记着的指令不清掉，屏幕上也没东西要改
            return Partial::Hold;
        }
        if let Some(word) = &self.segment.fired {
            return Partial::Type(after_command(text, word).to_string());
        }
        if self.segment.standalone.is_none() {
            let standalone = self.paused_enough();
            self.segment.standalone = Some(standalone);
            self.segment.open = standalone;
        }
        if self.segment.open && starts {
            if is_command {
                match &self.segment.pending {
                    Some((old, _)) if same_words(old, text) => {}
                    _ => self.segment.pending = Some((text.to_string(), now)),
                }
            } else {
                self.segment.pending = None;
            }
            return Partial::Hold;
        }
        self.segment.open = false;
        self.segment.pending = None;
        Partial::Type(text.to_string())
    }

    /// 停够 `settle` 了：交出要执行的那版中间结果（交出后这句记成「已执行」）
    pub fn due(&mut self, now: Instant) -> Option<String> {
        let ready = self
            .segment
            .pending
            .as_ref()
            .is_some_and(|(_, since)| now.duration_since(*since) >= self.settle);
        if !ready {
            return None;
        }
        let (spoken, _) = self.segment.pending.take()?;
        self.segment.fired = Some(spoken.clone());
        Some(spoken)
    }

    /// 这句定稿了。`is_command`：定稿整句就是开着的指令。之后开始新的一句
    pub fn finished(&mut self, final_text: &str, is_command: bool) -> Finish {
        let segment = std::mem::take(&mut self.segment);
        if let Some(word) = segment.fired {
            return Finish::Text(after_command(final_text, &word).to_string());
        }
        let open = segment.standalone.unwrap_or_else(|| self.paused_enough())
            && (segment.open || segment.standalone.is_none());
        if open && is_command {
            return Finish::Command(final_text.to_string());
        }
        if open
            && let Some((spoken, _)) = segment.pending
            && !contains_words(final_text, &spoken)
        {
            return Finish::Command(spoken);
        }
        Finish::Text(final_text.to_string())
    }
}

/// 只比字母、数字和汉字：「删除」和「删除。」是同一版
fn same_words(a: &str, b: &str) -> bool {
    letters(a) == letters(b)
}

fn letters(text: &str) -> String {
    text.chars().filter(|c| c.is_alphanumeric()).collect()
}

/// 定稿里还有没有说出来的那几个字（不管标点）
fn contains_words(text: &str, word: &str) -> bool {
    let word = letters(word);
    word.is_empty() || letters(text).contains(&word)
}

/// 去掉开头的指令词（按字母、数字、汉字对，不管标点），返回后面接着说的；开头不是它就原样返回
fn after_command<'a>(text: &'a str, word: &str) -> &'a str {
    let mut wanted = word.chars().filter(|c| c.is_alphanumeric()).peekable();
    if wanted.peek().is_none() {
        return text;
    }
    let mut cut = 0;
    for (i, c) in text.char_indices() {
        if wanted.peek().is_none() {
            break;
        }
        if !c.is_alphanumeric() {
            continue;
        }
        if wanted.next() != Some(c) {
            return text;
        }
        cut = i + c.len_utf8();
    }
    if wanted.peek().is_some() {
        return text;
    }
    text[cut..].trim_start_matches(|c: char| !c.is_alphanumeric())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAUSE: Duration = Duration::from_millis(1000);
    const SETTLE: Duration = Duration::from_millis(600);
    /// 测试里开着的指令
    const PHRASES: [&str; 3] = ["删除", "全部删除", "翻译成英文"];

    fn gate() -> CommandGate {
        CommandGate::new(PAUSE, SETTLE)
    }

    fn starts(text: &str) -> bool {
        let key = letters(text);
        !key.is_empty() && PHRASES.iter().any(|p| p.starts_with(&key))
    }

    fn is_command(text: &str) -> bool {
        PHRASES.contains(&letters(text).as_str())
    }

    /// 喂一串（毫秒, 中间结果），每 20 毫秒看一次停够了没有；返回每版怎么上屏，以及什么时候执行了什么
    fn run(
        gate: &mut CommandGate,
        events: &[(u64, &str)],
    ) -> (Vec<Partial>, Option<(u64, String)>) {
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        let mut shown = Vec::new();
        let mut fired = None;
        let last = events.last().map_or(0, |e| e.0) + 1000;
        let mut events = events.iter().peekable();
        for ms in (0..=last).step_by(20) {
            while let Some((t, text)) = events.peek() {
                if *t > ms {
                    break;
                }
                shown.push(gate.partial(text, starts(text), is_command(text), at(*t)));
                events.next();
            }
            if fired.is_none()
                && let Some(text) = gate.due(at(ms))
            {
                fired = Some((ms, text));
            }
        }
        (shown, fired)
    }

    #[test]
    fn a_command_said_on_its_own_after_a_pause_runs_without_being_typed() {
        let mut g = gate();
        g.voice(1_300);
        let (shown, fired) = run(&mut g, &[(0, "删"), (200, "删除"), (400, "删除。")]);
        assert!(shown.iter().all(|p| *p == Partial::Hold), "{shown:?}");
        // 「删除。」只比「删除」多了标点，不重新计时
        assert_eq!(fired, Some((800, "删除".to_owned())));
        // 定稿还是「删除」：已经执行过了，什么都不打
        assert_eq!(g.finished("删除。", true), Finish::Text(String::new()));
    }

    // 主人 2026-11-24：「今天吃什么，换行」一口气说的不该算；要先停够
    #[test]
    fn a_command_without_a_long_enough_pause_before_it_is_typed_as_text() {
        let mut g = gate();
        g.voice(400);
        let (shown, fired) = run(&mut g, &[(0, "删除")]);
        assert_eq!(shown, [Partial::Type("删除".into())]);
        assert_eq!(fired, None);
        assert_eq!(g.finished("删除。", true), Finish::Text("删除。".into()));
    }

    #[test]
    fn the_first_words_of_a_round_count_as_paused() {
        let mut g = gate();
        let (_, fired) = run(&mut g, &[(0, "全部删除")]);
        assert_eq!(fired, Some((600, "全部删除".to_owned())));
    }

    #[test]
    fn going_on_to_say_something_else_types_the_whole_sentence() {
        let mut g = gate();
        g.voice(2_000);
        let (shown, fired) = run(
            &mut g,
            &[(0, "删除"), (300, "删除这个"), (500, "删除这个文件")],
        );
        assert_eq!(
            shown,
            [
                Partial::Hold,
                Partial::Type("删除这个".into()),
                Partial::Type("删除这个文件".into())
            ]
        );
        assert_eq!(fired, None);
        assert_eq!(
            g.finished("删除这个文件。", false),
            Finish::Text("删除这个文件。".into())
        );
    }

    #[test]
    fn words_that_only_start_like_a_command_are_typed_once_they_differ() {
        let mut g = gate();
        g.voice(1_500);
        let (shown, _) = run(&mut g, &[(0, "全部"), (200, "全部的人")]);
        assert_eq!(shown, [Partial::Hold, Partial::Type("全部的人".into())]);
    }

    #[test]
    fn speech_after_an_executed_command_is_still_typed() {
        let mut g = gate();
        g.voice(1_200);
        let (shown, fired) = run(&mut g, &[(0, "删除"), (1_000, "删除今天天气")]);
        assert_eq!(fired, Some((600, "删除".to_owned())));
        assert_eq!(shown[1], Partial::Type("今天天气".into()));
        assert_eq!(
            g.finished("删除，今天天气很好。", true),
            Finish::Text("今天天气很好。".into())
        );
    }

    // 2026-11-23 微信：单独说「翻译成英文」，定稿变成「Translate into English.」
    // 2026-11-24 豆包：单独说的指令被吞，中间结果最后变成空的、定稿也是空的
    #[test]
    fn a_final_that_lost_or_rewrote_the_command_word_still_runs_the_command() {
        let mut g = gate();
        g.voice(1_500);
        run(&mut g, &[(0, "翻译成英文")]);
        // 没等到停够定稿就来了
        let mut g = gate();
        g.voice(1_500);
        let start = Instant::now();
        assert_eq!(g.partial("翻译成英文", true, true, start), Partial::Hold);
        assert_eq!(
            g.finished("Translate into English.", false),
            Finish::Command("翻译成英文".into())
        );
        let mut g = gate();
        g.voice(1_500);
        assert_eq!(g.partial("删除", true, true, start), Partial::Hold);
        assert_eq!(g.partial("", false, false, start), Partial::Hold);
        assert_eq!(g.finished("", false), Finish::Command("删除".into()));
    }

    #[test]
    fn a_final_that_arrives_without_partials_is_judged_by_the_pause_too() {
        let mut g = gate();
        g.voice(1_500);
        assert_eq!(g.finished("删除。", true), Finish::Command("删除。".into()));
        let mut g = gate();
        g.voice(300);
        assert_eq!(g.finished("删除。", true), Finish::Text("删除。".into()));
    }

    #[test]
    fn each_sentence_starts_fresh() {
        let mut g = gate();
        g.voice(1_500);
        run(&mut g, &[(0, "今天吃什么")]);
        assert_eq!(
            g.finished("今天吃什么。", false),
            Finish::Text("今天吃什么。".into())
        );
        // 停够了再单独说：算指令
        g.voice(1_100);
        let (_, fired) = run(&mut g, &[(0, "删除")]);
        assert_eq!(fired, Some((600, "删除".to_owned())));
    }
}
