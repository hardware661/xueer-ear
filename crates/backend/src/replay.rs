//! 一句话的音频留底，用来在连接断掉后重连重发（纯逻辑，可在 WSL 测）。
//! 主人 2026-11-15：网络抖一下整句话就没了，而且没有任何提示。
//!
//! 一句话最多留 `MAX_BYTES`（约几分钟的 Opus），超了就不再留底——那一句断线后不重发，直接报错。

/// 一句话留底的上限。Opus 16 kHz 语音每秒约 3 KB，2 MiB 约 10 分钟。
pub const MAX_BYTES: usize = 2 * 1024 * 1024;

#[derive(Default)]
pub struct Replay {
    chunks: Vec<Vec<u8>>,
    bytes: usize,
    overflowed: bool,
    retried: bool,
}

impl Replay {
    /// 记下刚上传的一块音频
    pub fn push(&mut self, chunk: &[u8]) {
        if self.overflowed {
            return;
        }
        if self.bytes + chunk.len() > MAX_BYTES {
            self.overflowed = true;
            self.chunks = Vec::new();
            return;
        }
        self.bytes += chunk.len();
        self.chunks.push(chunk.to_vec());
    }

    /// 连接出错了：这一句还能不能重连重发？每句只重试一次；留底超限的不行。
    /// 能的话返回要重发的全部音频块，并记下「已经重试过」。
    pub fn take_for_retry(&mut self) -> Option<Vec<Vec<u8>>> {
        if self.retried || self.overflowed {
            return None;
        }
        self.retried = true;
        Some(self.chunks.clone())
    }

    /// 连接出了 `error`：这句还没重试过，就用 `connect` 重连，再用 `upload` 把留底按顺序重发，
    /// 返回新连接；`connect` 返回 `Ok(None)`（重连时被取消）就原样返回。
    /// 这句已经重试过或留底超限，就把原来的 `error` 交回去。
    pub fn recover<L, E>(
        &mut self,
        error: E,
        connect: impl FnOnce() -> Result<Option<L>, E>,
        mut upload: impl FnMut(&mut L, &[u8]) -> Result<(), E>,
    ) -> Result<Option<L>, E> {
        let Some(chunks) = self.take_for_retry() else {
            return Err(error);
        };
        let Some(mut link) = connect()? else {
            return Ok(None);
        };
        for chunk in &chunks {
            upload(&mut link, chunk)?;
        }
        Ok(Some(link))
    }

    /// 这一句是不是已经重试过
    pub fn retried(&self) -> bool {
        self.retried
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_failure_replays_everything_in_order() {
        let mut r = Replay::default();
        r.push(b"head");
        r.push(b"one");
        r.push(b"two");
        assert_eq!(
            r.take_for_retry(),
            Some(vec![b"head".to_vec(), b"one".to_vec(), b"two".to_vec()])
        );
        assert!(r.retried());
    }

    #[test]
    fn audio_after_a_retry_is_still_kept_but_second_failure_gives_up() {
        let mut r = Replay::default();
        r.push(b"a");
        assert!(r.take_for_retry().is_some());
        r.push(b"b");
        assert_eq!(r.take_for_retry(), None, "每句只重试一次");
    }

    #[test]
    fn failure_before_any_audio_still_retries_once() {
        // 一开口就连不上：没有音频要重发，但值得再连一次
        let mut r = Replay::default();
        assert_eq!(r.take_for_retry(), Some(vec![]));
        assert_eq!(r.take_for_retry(), None);
    }

    #[test]
    fn recover_reconnects_and_resends_the_sentence_in_order() {
        let mut r = Replay::default();
        r.push(b"one");
        r.push(b"two");
        let link = r.recover(
            "断了",
            || Ok(Some(Vec::<Vec<u8>>::new())),
            |link, chunk| {
                link.push(chunk.to_vec());
                Ok(())
            },
        );
        assert_eq!(link, Ok(Some(vec![b"one".to_vec(), b"two".to_vec()])));
    }

    #[test]
    fn recover_gives_back_the_original_error_the_second_time() {
        let mut r = Replay::default();
        let first: Result<Option<()>, &str> = r.recover("第一次", || Ok(Some(())), |_, _| Ok(()));
        assert_eq!(first, Ok(Some(())));
        let second: Result<Option<()>, &str> =
            r.recover("第二次", || panic!("第二次失败不该再连"), |_, _| Ok(()));
        assert_eq!(second, Err("第二次"));
    }

    #[test]
    fn recover_passes_on_cancel_and_resend_failures() {
        // 重连时主人取消了：没有新连接，也不算错
        let mut r = Replay::default();
        r.push(b"x");
        let cancelled: Result<Option<()>, &str> =
            r.recover("断了", || Ok(None), |_, _| panic!("没连上不该上传"));
        assert_eq!(cancelled, Ok(None));
        // 重连还是连不上、或者重发又断了：报新的错
        let mut r = Replay::default();
        r.push(b"x");
        let failed: Result<Option<()>, &str> =
            r.recover("断了", || Err("还是连不上"), |_, _| Ok(()));
        assert_eq!(failed, Err("还是连不上"));
        let mut r = Replay::default();
        r.push(b"x");
        let failed: Result<Option<()>, &str> =
            r.recover("断了", || Ok(Some(())), |_, _| Err("重发又断了"));
        assert_eq!(failed, Err("重发又断了"));
    }

    #[test]
    fn oversized_sentence_is_not_replayed() {
        let mut r = Replay::default();
        r.push(&vec![0; MAX_BYTES]);
        r.push(b"x");
        assert_eq!(r.take_for_retry(), None);
    }
}
