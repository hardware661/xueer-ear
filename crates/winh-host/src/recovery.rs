//! 快捷键听写出错后的处理（纯逻辑，可在 WSL 测）：归类、跟主人说什么。
//! 重连重发在识别引擎里做（每句一次，见 backend 的 `replay.rs`）；到这里的错都是重试过还不行的。
//! 主人 2026-11-15 对照同类工具后定：出了错不能没反应。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// 网络或云端连接问题
    Network,
    /// 麦克风打不开、被占用
    Microphone,
    /// 其他
    Other,
}

/// 按报错文字归类。报错来自 std / tungstenite / 我们自己的中文提示。
pub fn classify(error: &str) -> Kind {
    let e = error.to_lowercase();
    const MICROPHONE: &[&str] = &["麦克风", "audio capture", "microphone", "wasapi"];
    const NETWORK: &[&str] = &[
        "timed out",
        "timeout",
        "connection",
        "io error",
        "os error",
        "tls",
        "http ",
        "lookup",
        "address",
        "ipv4",
        "dns",
        "socket",
        "网络",
        "连接",
        "主机",
    ];
    if MICROPHONE.iter().any(|k| e.contains(k)) {
        Kind::Microphone
    } else if NETWORK.iter().any(|k| e.contains(k)) {
        Kind::Network
    } else {
        Kind::Other
    }
}

/// 给主人看的一句话（面板和控制面板显示）。
pub fn message(kind: Kind) -> &'static str {
    match kind {
        Kind::Network => "网络不通，这句没识别出来",
        Kind::Microphone => "麦克风打不开，请检查设备",
        Kind::Other => "这次出错了，请再说一遍",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_errors_seen_in_the_log_are_network() {
        for e in [
            "connection timed out",
            "IO error: 重叠 I/O 操作在进行中。 (os error 997)",
            "WeType speech connection closed",
            "IO error: 远程主机强迫关闭了一个现有的连接。 (os error 10054)",
            "failed to lookup address information: 不知道这样的主机。",
            "WeType endpoint has no IPv4 address",
            "TLS error: native-tls error",
            "WeType HTTP 502",
        ] {
            assert_eq!(classify(e), Kind::Network, "{e}");
        }
    }

    #[test]
    fn microphone_errors() {
        for e in [
            "audio capture failed to initialize",
            "无法打开麦克风",
            "麦克风 ID 无效",
            "麦克风打不开：找不到元素。 (0x80070490)",
        ] {
            assert_eq!(classify(e), Kind::Microphone, "{e}");
        }
        assert_eq!(classify("dictation text exceeds limit"), Kind::Other);
    }

    #[test]
    fn messages_are_short_and_say_what_to_do() {
        for kind in [Kind::Network, Kind::Microphone, Kind::Other] {
            let m = message(kind);
            assert!(!m.is_empty() && m.chars().count() <= 24, "{m}");
        }
        assert!(message(Kind::Network).contains("网络"));
        assert!(message(Kind::Microphone).contains("麦克风"));
    }
}
