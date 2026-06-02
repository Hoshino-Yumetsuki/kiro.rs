//! Anthropic usage 计算工具
//!
//! 集中放置 input/output token 的计算逻辑，避免在多个调用方各自维护一份重复实现。

/// 计算 Anthropic 计费口径的 input_tokens。
///
/// 当存在缓存活动（creation + read > 0）时，固定返回 1，与 Anthropic 官方行为一致：
/// `input_tokens` = 1, `cache_creation + cache_read` = total - 1。
///
/// 无缓存活动时，返回 total 本身。
pub fn billed_input_tokens(total: i32, creation: i32, read: i32) -> i32 {
    if creation > 0 || read > 0 {
        1
    } else {
        total.max(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn billed_returns_1_when_cache_active() {
        assert_eq!(billed_input_tokens(1000, 50, 800), 1);
        assert_eq!(billed_input_tokens(78526, 38033, 40473), 1);
    }

    #[test]
    fn billed_returns_total_when_no_cache() {
        assert_eq!(billed_input_tokens(1000, 0, 0), 1000);
    }

    #[test]
    fn billed_returns_1_with_only_creation() {
        assert_eq!(billed_input_tokens(5000, 4999, 0), 1);
    }

    #[test]
    fn billed_returns_1_with_only_read() {
        assert_eq!(billed_input_tokens(5000, 0, 4999), 1);
    }

    #[test]
    fn billed_handles_negative_total() {
        assert_eq!(billed_input_tokens(-5, 0, 0), 0);
    }
}
