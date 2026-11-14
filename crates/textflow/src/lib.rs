//! 识别文字的后处理。所有函数都是纯函数：同一段输入、同一份配置，永远得到同一段输出，
//! 这样 Win+H 边说边改时，对同一句的多次修订结果前后一致。

mod fillers;

pub use fillers::strip_fillers;
mod numbers;
pub use numbers::normalize_numbers;
