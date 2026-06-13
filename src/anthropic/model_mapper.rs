//! 模型映射器：将 Anthropic 模型名映射到 Kiro 上游模型 ID
//!
//! 支持从配置文件加载模型列表，也可使用内置默认列表。
//! 映射策略：精确匹配优先，Claude 系列回退到子串匹配。

use std::collections::HashMap;

use super::types::ModelInfo;
use crate::model::config::ModelConfig;

/// 模型映射器
///
/// 包含两个数据结构：
/// - `mappings`: Anthropic 模型 ID（小写）→ Kiro 上游模型 ID
/// - `model_infos`: 对外暴露的模型列表（Anthropic API 格式）
#[derive(Debug, Clone)]
pub struct ModelMapper {
    /// 小写 Anthropic ID → Kiro 上游 ID
    mappings: HashMap<String, String>,
    /// 对外暴露的模型列表
    model_infos: Vec<ModelInfo>,
}

impl ModelMapper {
    /// 从配置模型列表构建映射器
    pub fn from_config(models: &[ModelConfig]) -> Self {
        let mut mappings = HashMap::new();
        let mut model_infos = Vec::new();

        for m in models {
            // 精确匹配条目：小写 ID → kiro_model_id
            mappings.insert(m.id.to_lowercase(), m.kiro_model_id.clone());

            model_infos.push(ModelInfo {
                id: m.id.clone(),
                model_type: "model".to_string(),
                display_name: m.display_name.clone(),
                created_at: m.created_at,
            });
        }

        Self {
            mappings,
            model_infos,
        }
    }

    /// 模型映射：将 Anthropic 模型名映射到 Kiro 上游模型 ID
    ///
    /// 策略：
    /// 1. 精确匹配（不区分大小写）
    /// 2. Claude 子串回退（兼容旧版本/自定义模型名）
    pub fn map_model(&self, model: &str) -> Option<String> {
        let lower = model.to_lowercase();

        // 1. 精确匹配
        if let Some(kiro_id) = self.mappings.get(&lower) {
            return Some(kiro_id.clone());
        }

        // 2. Claude 子串回退（兼容 claude-sonnet-4-20250514 等旧格式）
        if lower.contains("sonnet") {
            if lower.contains("4-6") || lower.contains("4.6") {
                return self.mappings.get("claude-sonnet-4-6").cloned();
            } else if lower.contains("4-0") || lower.contains("4.0") {
                return self.mappings.get("claude-sonnet-4-0-20250904").cloned();
            } else {
                return self.mappings.get("claude-sonnet-4-5-20250929").cloned();
            }
        } else if lower.contains("opus") {
            if lower.contains("4-5") || lower.contains("4.5") {
                return self.mappings.get("claude-opus-4-5-20251101").cloned();
            } else if lower.contains("4-7") || lower.contains("4.7") {
                return self.mappings.get("claude-opus-4-7").cloned();
            } else if lower.contains("4-8") || lower.contains("4.8") {
                return self.mappings.get("claude-opus-4-8").cloned();
            } else {
                return self.mappings.get("claude-opus-4-6").cloned();
            }
        } else if lower.contains("haiku") {
            return self.mappings.get("claude-haiku-4-5-20251001").cloned();
        }

        None
    }

    /// 获取对外暴露的模型列表
    pub fn model_infos(&self) -> &[ModelInfo] {
        &self.model_infos
    }
}

impl Default for ModelMapper {
    fn default() -> Self {
        Self::from_config(&default_models())
    }
}

/// 内置默认模型列表（来自 Kiro 官方文档 https://kiro.dev/docs/models）
///
/// 包含 Claude 系列和非 Claude 模型。
/// `kiro_model_id` 是发送给 Kiro 上游的实际模型标识。
pub fn default_models() -> Vec<ModelConfig> {
    vec![
        // === Claude 系列 ===
        ModelConfig {
            id: "claude-sonnet-4-6".to_string(),
            display_name: "Claude Sonnet 4.6".to_string(),
            created_at: 1739836800, // 2025-02-17
            kiro_model_id: "claude-sonnet-4.6".to_string(),
        },
        ModelConfig {
            id: "claude-sonnet-4-5-20250929".to_string(),
            display_name: "Claude Sonnet 4.5".to_string(),
            created_at: 1727568000, // 2025-09-29
            kiro_model_id: "claude-sonnet-4.5".to_string(),
        },
        ModelConfig {
            id: "claude-sonnet-4-0-20250904".to_string(),
            display_name: "Claude Sonnet 4.0".to_string(),
            created_at: 1725408000, // 2025-09-04
            kiro_model_id: "claude-sonnet-4.0".to_string(),
        },
        ModelConfig {
            id: "claude-opus-4-5-20251101".to_string(),
            display_name: "Claude Opus 4.5".to_string(),
            created_at: 1730419200, // 2025-11-01
            kiro_model_id: "claude-opus-4.5".to_string(),
        },
        ModelConfig {
            id: "claude-opus-4-6".to_string(),
            display_name: "Claude Opus 4.6".to_string(),
            created_at: 1738713600, // 2026-02-05
            kiro_model_id: "claude-opus-4.6".to_string(),
        },
        ModelConfig {
            id: "claude-opus-4-7".to_string(),
            display_name: "Claude Opus 4.7".to_string(),
            created_at: 1744934400, // 2026-04-16 (Experimental)
            kiro_model_id: "claude-opus-4.7".to_string(),
        },
        ModelConfig {
            id: "claude-opus-4-8".to_string(),
            display_name: "Claude Opus 4.8".to_string(),
            created_at: 1748390400, // 2026-05-28 (Experimental)
            kiro_model_id: "claude-opus-4.8".to_string(),
        },
        ModelConfig {
            id: "claude-haiku-4-5-20251001".to_string(),
            display_name: "Claude Haiku 4.5".to_string(),
            created_at: 1727740800, // 2025-10-01
            kiro_model_id: "claude-haiku-4.5".to_string(),
        },
        // === 非 Claude 模型 ===
        ModelConfig {
            id: "deepseek-3-2".to_string(),
            display_name: "DeepSeek 3.2".to_string(),
            created_at: 1739145600, // 2026-02-10 (Experimental)
            kiro_model_id: "deepseek-3.2".to_string(),
        },
        ModelConfig {
            id: "minimax-m2-5".to_string(),
            display_name: "MiniMax M2.5".to_string(),
            created_at: 1742256000, // 2026-03-18 (Experimental)
            kiro_model_id: "minimax-m2.5".to_string(),
        },
        ModelConfig {
            id: "glm-5".to_string(),
            display_name: "GLM-5".to_string(),
            created_at: 1743379200, // 2026-03-31 (Experimental)
            kiro_model_id: "glm-5".to_string(),
        },
        ModelConfig {
            id: "minimax-m2-1".to_string(),
            display_name: "MiniMax M2.1".to_string(),
            created_at: 1739145600, // 2026-02-10 (Experimental)
            kiro_model_id: "minimax-m2.1".to_string(),
        },
        ModelConfig {
            id: "qwen3-coder-next".to_string(),
            display_name: "Qwen3 Coder Next".to_string(),
            created_at: 1739145600, // 2026-02-10 (Experimental)
            kiro_model_id: "qwen3-coder-next".to_string(),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exact_match() {
        let mapper = ModelMapper::default();
        assert_eq!(
            mapper.map_model("claude-sonnet-4-6"),
            Some("claude-sonnet-4.6".to_string())
        );
        assert_eq!(
            mapper.map_model("claude-opus-4-6"),
            Some("claude-opus-4.6".to_string())
        );
        assert_eq!(
            mapper.map_model("deepseek-3-2"),
            Some("deepseek-3.2".to_string())
        );
    }

    #[test]
    fn test_case_insensitive_exact_match() {
        let mapper = ModelMapper::default();
        assert_eq!(
            mapper.map_model("Claude-Sonnet-4-6"),
            Some("claude-sonnet-4.6".to_string())
        );
    }

    #[test]
    fn test_claude_substring_fallback() {
        let mapper = ModelMapper::default();
        // 旧格式 sonnet → 默认回退到 sonnet 4.5
        assert_eq!(
            mapper.map_model("claude-sonnet-4-20250514"),
            Some("claude-sonnet-4.5".to_string())
        );
        assert_eq!(
            mapper.map_model("claude-3-5-sonnet-20241022"),
            Some("claude-sonnet-4.5".to_string())
        );
        // sonnet 4.6
        assert_eq!(
            mapper.map_model("claude-sonnet-4.6"),
            Some("claude-sonnet-4.6".to_string())
        );
        // opus 默认回退到 4.6
        assert_eq!(
            mapper.map_model("claude-opus-4-20250514"),
            Some("claude-opus-4.6".to_string())
        );
        assert_eq!(
            mapper.map_model("claude-opus-4-20260206"),
            Some("claude-opus-4.6".to_string())
        );
        // opus 4.5
        assert_eq!(
            mapper.map_model("claude-opus-4-5-20250514"),
            Some("claude-opus-4.5".to_string())
        );
        // haiku
        assert_eq!(
            mapper.map_model("claude-haiku-4-20250514"),
            Some("claude-haiku-4.5".to_string())
        );
    }

    #[test]
    fn test_unknown_model() {
        let mapper = ModelMapper::default();
        assert_eq!(mapper.map_model("gpt-4"), None);
        assert_eq!(mapper.map_model("unknown-model"), None);
    }

    #[test]
    fn test_versioned_entries_from_models_endpoint() {
        let mapper = ModelMapper::default();
        let supported_models = [
            ("claude-sonnet-4-6", "claude-sonnet-4.6"),
            ("claude-sonnet-4-5-20250929", "claude-sonnet-4.5"),
            ("claude-opus-4-5-20251101", "claude-opus-4.5"),
            ("claude-opus-4-6", "claude-opus-4.6"),
            ("claude-opus-4-7", "claude-opus-4.7"),
            ("claude-opus-4-8", "claude-opus-4.8"),
            ("claude-haiku-4-5-20251001", "claude-haiku-4.5"),
        ];
        for (input, expected) in supported_models {
            assert_eq!(
                mapper.map_model(input),
                Some(expected.to_string()),
                "{input}"
            );
        }
    }

    #[test]
    fn test_non_claude_models() {
        let mapper = ModelMapper::default();
        assert_eq!(
            mapper.map_model("deepseek-3-2"),
            Some("deepseek-3.2".to_string())
        );
        assert_eq!(
            mapper.map_model("minimax-m2-5"),
            Some("minimax-m2.5".to_string())
        );
        assert_eq!(mapper.map_model("glm-5"), Some("glm-5".to_string()));
        assert_eq!(
            mapper.map_model("minimax-m2-1"),
            Some("minimax-m2.1".to_string())
        );
        assert_eq!(
            mapper.map_model("qwen3-coder-next"),
            Some("qwen3-coder-next".to_string())
        );
    }

    #[test]
    fn test_model_infos_count() {
        let mapper = ModelMapper::default();
        // 8 Claude + 5 non-Claude = 13 models
        assert_eq!(mapper.model_infos().len(), 13);
    }

    #[test]
    fn test_custom_config() {
        let custom = vec![ModelConfig {
            id: "my-custom-model".to_string(),
            display_name: "Custom Model".to_string(),
            created_at: 1700000000,
            kiro_model_id: "custom-upstream-id".to_string(),
        }];
        let mapper = ModelMapper::from_config(&custom);
        assert_eq!(
            mapper.map_model("my-custom-model"),
            Some("custom-upstream-id".to_string())
        );
        assert_eq!(mapper.model_infos().len(), 1);
        // Claude fallback still works for substring matches even with custom config
        // but only if there are matching entries in the mappings
        assert_eq!(mapper.map_model("claude-sonnet-4-6"), None);
    }
}
