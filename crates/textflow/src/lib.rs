//! 识别文字的后处理。所有函数都是纯函数：同一段输入、同一份配置，永远得到同一段输出，
//! 这样 Win+H 边说边改时，对同一句的多次修订结果前后一致。

mod fillers;

pub use fillers::strip_fillers;
mod numbers;
pub use numbers::normalize_numbers;
mod punct;
pub use punct::{PunctOptions, apply_punctuation};
mod commands;
pub use commands::{CommandOptions, VoiceCommand, parse_command, parse_trailing_command};
mod hotwords;
pub use hotwords::{Hotwords, add_hotword_line};
mod pipeline;
pub use pipeline::{Options, Output, join_for_terminal, process, terminal_append};
