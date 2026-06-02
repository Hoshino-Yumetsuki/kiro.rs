//! Anthropic usage 计算工具
//!
//! 集中放置 input/output token 的计算逻辑，避免在多个调用方各自维护一份重复实现。

/// 计算 Anthropic 计费口径的 input_tokens：`total - creation - read`，并下取 0。
///
/// Anthropic 协议中 `input_tokens` 仅代表"非缓存"部分；缓存创建和缓存读取通过
/// 独立的 `cache_creation_input_tokens` / `cache_read_input_tokens` 字段表达。
pub fn billed_input_tokens(total: i32, creation: i32, read: i32) -> i32 {
    total.saturating_sub(creation).saturating_sub(read).max(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn billed_subtracts_creation_and_read() {
        assert_eq!(billed_input_tokens(1000, 50, 800), 150);
    }

    #[test]
    fn billed_floors_at_zero() {
        assert_eq!(billed_input_tokens(10, 50, 50), 0);
    }

    #[test]
    fn billed_handles_negative_total() {
        assert_eq!(billed_input_tokens(-5, 0, 0), 0);
    }
}
