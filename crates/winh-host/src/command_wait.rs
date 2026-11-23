//! 中间结果里说了语音指令：停一小会儿没再变就执行，不等云端定稿。
//!
//! 豆包会把接在话后面单独说的「撤回」自己吞掉：中间结果是「撤回」「撤回。」「撤回」，最后变成空的，
//! 定稿也是空的（2026-11-24 主人真机：撤回没反应；合成语音复现）。所以这里
//! - 只看说的字变没变，标点不算（「撤回」→「撤回。」不重新计时）；
//! - 空的中间结果不清掉记着的指令（那是云端把指令词吞了，不是改说了别的）；
//! - 定稿里找不到指令词（被吞了、被改写了）时，按记着的那版中间结果执行。
use std::time::{Duration, Instant};

#[derive(Debug, Default)]
pub struct CommandWait {
    /// （说出指令的那版中间结果, 从什么时候起没变）
    pending: Option<(String, Instant)>,
}

impl CommandWait {
    /// 新的一版中间结果。`is_command`：这版整句只有指令，或者句尾是能接在句尾的指令
    pub fn partial(&mut self, text: &str, is_command: bool, now: Instant) {
        if text.trim().is_empty() {
            return;
        }
        if !is_command {
            self.pending = None;
            return;
        }
        match &self.pending {
            Some((old, _)) if same_words(old, text) => {}
            _ => self.pending = Some((text.to_string(), now)),
        }
    }

    /// 停够 `settle` 了：交出要执行的那版中间结果（交出后就不再记着）
    pub fn due(&mut self, settle: Duration, now: Instant) -> Option<String> {
        let ready = self
            .pending
            .as_ref()
            .is_some_and(|(_, since)| now.duration_since(*since) >= settle);
        if ready {
            self.pending.take().map(|(text, _)| text)
        } else {
            None
        }
    }

    /// 这句定稿了。定稿里找不到说过的指令词（云端吞了或改写了）就交出记着的那版，按它执行；
    /// 找得到就交给定稿照常处理。这句记着的指令都清掉
    pub fn finished(&mut self, final_text: &str) -> Option<String> {
        let (spoken, _) = self.pending.take()?;
        (!crate::typing::keeps_last_clause(&spoken, final_text)).then_some(spoken)
    }
}

/// 只比字母、数字和汉字：「撤回」和「撤回。」、「今天吃什么，发送」和「今天吃什么？发送。」是同一版
fn same_words(a: &str, b: &str) -> bool {
    let key = |s: &str| {
        s.chars()
            .filter(|c| c.is_alphanumeric())
            .collect::<String>()
    };
    key(a) == key(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SETTLE: Duration = Duration::from_millis(600);

    /// 一串（毫秒, 中间结果, 是不是指令）喂进去，返回停够时交出的指令和时间
    fn run(events: &[(u64, &str, bool)]) -> (CommandWait, Option<(u64, String)>) {
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        let mut wait = CommandWait::default();
        let mut fired = None;
        let last = events.last().map_or(0, |e| e.0);
        let mut events = events.iter().peekable();
        // 每 20 毫秒看一次（听写时音量事件大约就这么密）
        for ms in (0..=last).step_by(20) {
            while let Some((t, text, is_command)) = events.peek() {
                if *t > ms {
                    break;
                }
                wait.partial(text, *is_command, at(*t));
                events.next();
            }
            if fired.is_none()
                && let Some(text) = wait.due(SETTLE, at(ms))
            {
                fired = Some((ms, text));
            }
        }
        (wait, fired)
    }

    // 2026-11-24 合成语音实测豆包：一句话后面单独说「撤回」
    #[test]
    fn doubao_swallowing_the_word_after_punctuation_flips_still_runs_the_command() {
        // 「撤回。」只比「撤回」多了标点，在 600 ms 之内来，原来会重新计时；后面又来个空的，原来直接清掉
        let (mut wait, fired) = run(&[
            (0, "撤", false),
            (150, "撤回", true),
            (500, "撤回。", true),
            (500, "撤回", true),
            (700, "", false),
        ]);
        assert_eq!(fired, None, "还没停够 600 ms");
        // 定稿是空的：云端把指令词吞了，按记着的中间结果执行
        assert_eq!(wait.finished("").as_deref(), Some("撤回"));
    }

    #[test]
    fn a_command_that_stays_put_runs_after_the_settle_time() {
        let (_, fired) = run(&[
            (0, "撤回", true),
            (400, "撤回。", true),
            (900, "撤回。", true),
        ]);
        assert_eq!(fired, Some((600, "撤回".to_owned())), "标点变了不重新计时");
    }

    #[test]
    fn an_empty_partial_keeps_the_command_and_it_still_runs() {
        let (_, fired) = run(&[(0, "撤回", true), (300, "", false), (800, "", false)]);
        assert_eq!(fired, Some((600, "撤回".to_owned())));
    }

    #[test]
    fn going_on_to_say_something_else_cancels_the_command() {
        let (mut wait, fired) = run(&[
            (0, "删除", true),
            (300, "删除这个文件", false),
            (1000, "删除这个文件", false),
        ]);
        assert_eq!(fired, None);
        assert_eq!(wait.finished("删除这个文件。"), None);
    }

    #[test]
    fn a_final_that_keeps_the_command_word_is_handled_by_the_final_itself() {
        let (mut wait, _) = run(&[(0, "撤回", true), (200, "撤回。", true)]);
        assert_eq!(wait.finished("撤回。"), None);
        // 句尾指令：定稿还带着「发送」
        let (mut wait, _) = run(&[(0, "今天吃什么，发送", true)]);
        assert_eq!(wait.finished("今天吃什么？发送。"), None);
    }

    #[test]
    fn a_trailing_command_the_cloud_dropped_from_the_final_is_run_from_the_partial() {
        let (mut wait, _) = run(&[
            (0, "今天吃什么，发送", true),
            (300, "今天吃什么？发送。", true),
        ]);
        assert_eq!(
            wait.finished("今天吃什么？").as_deref(),
            Some("今天吃什么，发送")
        );
    }

    #[test]
    fn a_changed_command_restarts_the_wait() {
        let (_, fired) = run(&[
            (0, "删除", true),
            (400, "删除上一句", true),
            (1100, "删除上一句", true),
        ]);
        assert_eq!(fired, Some((1000, "删除上一句".to_owned())));
    }
}
