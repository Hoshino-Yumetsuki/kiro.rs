//! 工具定义压缩模块
//!
//! 提供两级工具定义压缩，可独立使用：
//! 1. `simplify_tool_schemas()` — 简化 `input_schema`：移除非必要字段（description 等），仅保留结构骨架
//! 2. `truncate_tool_descriptions()` — 按目标字符数截断每个工具的 description
//!
//! 在自适应压缩循环中，先尝试 schema 简化（一次性、低损），
//! 若仍超限再逐步降低 description 截断阈值。

use crate::kiro::model::requests::tool::{InputSchema, Tool as KiroTool, ToolSpecification};
use crate::model::config::CompressionConfig;

/// 如果工具定义总大小超过阈值，执行压缩（旧接口，保留兼容）
///
/// 返回压缩后的工具列表（如果未超阈值则原样返回）
#[allow(dead_code)]
pub fn compress_tools_if_needed(tools: &[KiroTool], config: &CompressionConfig) -> Vec<KiroTool> {
    if !config.tool_definition_compression {
        return tools.to_vec();
    }

    // 旧阈值逻辑保留用于测试兼容（默认 20KB）
    let threshold = 20 * 1024;
    let total_size = estimate_tools_size(tools);
    if total_size <= threshold {
        return tools.to_vec();
    }

    tracing::info!(
        total_size,
        threshold,
        tool_count = tools.len(),
        "工具定义超过阈值，开始压缩"
    );

    // 第一步：简化 input_schema
    let mut compressed: Vec<KiroTool> = tools.iter().map(simplify_schema).collect();

    let size_after_schema = estimate_tools_size(&compressed);
    if size_after_schema <= threshold {
        tracing::info!(
            original_size = total_size,
            compressed_size = size_after_schema,
            "schema 简化后已低于阈值"
        );
        return compressed;
    }
    // 第二步：按比例截断 description（基于字节大小）
    let ratio = threshold as f64 / size_after_schema as f64;
    let min_description_chars = config.tool_definition_min_description_chars;
    for tool in &mut compressed {
        let desc = &tool.tool_specification.description;
        let target_bytes = (desc.len() as f64 * ratio) as usize;
        // 最短保留配置指定字符数对应的字节数
        let min_bytes = desc
            .char_indices()
            .nth(min_description_chars)
            .map(|(idx, _)| idx)
            .unwrap_or(desc.len());
        let target_bytes = target_bytes.max(min_bytes);
        if desc.len() > target_bytes {
            // UTF-8 安全截断：找到不超过 target_bytes 的最大字符边界
            let truncate_at = desc
                .char_indices()
                .take_while(|(idx, _)| *idx <= target_bytes)
                .last()
                .map(|(idx, ch)| idx + ch.len_utf8())
                .unwrap_or(0);
            tool.tool_specification.description = desc[..truncate_at].to_string();
        }
    }

    let final_size = estimate_tools_size(&compressed);
    tracing::info!(
        original_size = total_size,
        after_schema = size_after_schema,
        final_size,
        "工具压缩完成"
    );

    compressed
}

/// 对工具列表执行 schema 简化（in-place）
///
/// 移除 input_schema 中 properties 内部的 description、examples 等非结构字段，
/// 仅保留 type、required、additionalProperties、enum、items 等骨架信息。
///
/// 返回节省的估算字节数。这是一次性操作，重复调用无额外效果。
pub fn simplify_tool_schemas(tools: &mut Vec<KiroTool>) -> usize {
    let before = estimate_tools_size(tools);
    *tools = tools.iter().map(simplify_schema).collect();
    let after = estimate_tools_size(tools);
    let saved = before.saturating_sub(after);
    if saved > 0 {
        tracing::info!(
            before,
            after,
            saved,
            tool_count = tools.len(),
            "自适应压缩：schema 简化完成"
        );
    }
    saved
}

/// 将每个工具的 description 截断到指定最大字符数（in-place）
///
/// 返回节省的估算字节数。
/// `max_chars` 为每个工具 description 允许的最大字符数。
/// `min_chars` 为截断下限，不会低于此值。
pub fn truncate_tool_descriptions(
    tools: &mut [KiroTool],
    max_chars: usize,
    min_chars: usize,
) -> usize {
    let effective_max = max_chars.max(min_chars);
    let mut total_saved = 0usize;
    for tool in tools.iter_mut() {
        let desc = &tool.tool_specification.description;
        let char_count = desc.chars().count();
        if char_count <= effective_max {
            continue;
        }
        // 找到第 effective_max 个字符的字节偏移
        let truncate_at = desc
            .char_indices()
            .nth(effective_max)
            .map(|(idx, _)| idx)
            .unwrap_or(desc.len());
        if truncate_at < desc.len() {
            total_saved += desc.len() - truncate_at;
            tool.tool_specification.description = desc[..truncate_at].to_string();
        }
    }
    if total_saved > 0 {
        tracing::info!(
            max_chars = effective_max,
            saved = total_saved,
            "自适应压缩：工具描述截断完成"
        );
    }
    total_saved
}

/// 估算工具列表的总序列化大小（字节）
pub fn estimate_tools_size(tools: &[KiroTool]) -> usize {
    tools
        .iter()
        .map(|t| {
            let spec = &t.tool_specification;
            spec.name.len()
                + spec.description.len()
                + serde_json::to_string(&spec.input_schema.json)
                    .map(|s| s.len())
                    .unwrap_or(0)
        })
        .sum()
}

/// 简化工具的 input_schema
///
/// 保留结构骨架（type, properties 的 key 和 type, required），
/// 移除 properties 内部的 description、examples 等非必要字段
fn simplify_schema(tool: &KiroTool) -> KiroTool {
    let schema = &tool.tool_specification.input_schema.json;
    let simplified = simplify_json_schema(schema);

    KiroTool {
        tool_specification: ToolSpecification {
            name: tool.tool_specification.name.clone(),
            description: tool.tool_specification.description.clone(),
            input_schema: InputSchema::from_json(simplified),
        },
    }
}

/// 递归简化 JSON Schema
fn simplify_json_schema(schema: &serde_json::Value) -> serde_json::Value {
    let Some(obj) = schema.as_object() else {
        return schema.clone();
    };

    let mut result = serde_json::Map::new();

    // 保留顶层结构字段
    for key in &["$schema", "type", "required", "additionalProperties"] {
        if let Some(v) = obj.get(*key) {
            result.insert(key.to_string(), v.clone());
        }
    }

    // 简化 properties：仅保留每个属性的 type
    if let Some(serde_json::Value::Object(props)) = obj.get("properties") {
        let mut simplified_props = serde_json::Map::new();
        for (name, prop_schema) in props {
            if let Some(prop_obj) = prop_schema.as_object() {
                let mut simplified_prop = serde_json::Map::new();
                // 保留 type
                if let Some(ty) = prop_obj.get("type") {
                    simplified_prop.insert("type".to_string(), ty.clone());
                }
                // 递归简化嵌套 properties（如 object 类型）
                if let Some(nested_props) = prop_obj.get("properties") {
                    // 构造完整的子 schema，保留 required 和 additionalProperties
                    let mut nested_schema = serde_json::Map::new();
                    nested_schema.insert(
                        "type".to_string(),
                        serde_json::Value::String("object".to_string()),
                    );
                    nested_schema.insert("properties".to_string(), nested_props.clone());
                    if let Some(req) = prop_obj.get("required") {
                        nested_schema.insert("required".to_string(), req.clone());
                    }
                    if let Some(ap) = prop_obj.get("additionalProperties") {
                        nested_schema.insert("additionalProperties".to_string(), ap.clone());
                    }
                    let nested = simplify_json_schema(&serde_json::Value::Object(nested_schema));
                    if let Some(np) = nested.get("properties") {
                        simplified_prop.insert("properties".to_string(), np.clone());
                    }
                    if let Some(req) = nested.get("required") {
                        simplified_prop.insert("required".to_string(), req.clone());
                    }
                    if let Some(ap) = nested.get("additionalProperties") {
                        simplified_prop.insert("additionalProperties".to_string(), ap.clone());
                    }
                }
                // 保留 items（数组类型）
                if let Some(items) = prop_obj.get("items") {
                    simplified_prop.insert("items".to_string(), simplify_json_schema(items));
                }
                // 保留 enum
                if let Some(e) = prop_obj.get("enum") {
                    simplified_prop.insert("enum".to_string(), e.clone());
                }
                simplified_props.insert(name.clone(), serde_json::Value::Object(simplified_prop));
            } else {
                simplified_props.insert(name.clone(), prop_schema.clone());
            }
        }
        result.insert(
            "properties".to_string(),
            serde_json::Value::Object(simplified_props),
        );
    }

    serde_json::Value::Object(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_tool(name: &str, desc: &str, schema: serde_json::Value) -> KiroTool {
        KiroTool {
            tool_specification: ToolSpecification {
                name: name.to_string(),
                description: desc.to_string(),
                input_schema: InputSchema::from_json(schema),
            },
        }
    }

    #[test]
    fn test_no_compression_under_threshold() {
        let tools = vec![make_tool(
            "test",
            "A short description",
            serde_json::json!({"type": "object", "properties": {}}),
        )];
        let result = compress_tools_if_needed(&tools, &CompressionConfig::default());
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0].tool_specification.description,
            "A short description"
        );
    }

    #[test]
    fn test_tool_definition_compression_can_be_disabled() {
        let long_desc = "x".repeat(2000);
        let tools: Vec<KiroTool> = (0..15)
            .map(|i| {
                make_tool(
                    &format!("tool_{}", i),
                    &long_desc,
                    serde_json::json!({
                        "type": "object",
                        "properties": {
                            "param": {"type": "string", "description": "A very long parameter description"}
                        }
                    }),
                )
            })
            .collect();
        let mut config = CompressionConfig::default();
        config.tool_definition_compression = false;

        let result = compress_tools_if_needed(&tools, &config);

        assert_eq!(estimate_tools_size(&result), estimate_tools_size(&tools));
        assert_eq!(result[0].tool_specification.description, long_desc);
    }

    #[test]
    fn test_tool_definition_threshold_zero_disables_compression() {
        let tools = vec![make_tool(
            "test",
            &"x".repeat(2000),
            serde_json::json!({"type": "object", "properties": {}}),
        )];
        let mut config = CompressionConfig::default();
        config.tool_definition_compression = false;

        let result = compress_tools_if_needed(&tools, &config);

        assert_eq!(estimate_tools_size(&result), estimate_tools_size(&tools));
    }

    #[test]
    fn test_compression_triggers_over_threshold() {
        // 创建大量工具使总大小超过 20KB
        let long_desc = "x".repeat(2000);
        let tools: Vec<KiroTool> = (0..15)
            .map(|i| {
                make_tool(
                    &format!("tool_{}", i),
                    &long_desc,
                    serde_json::json!({
                        "type": "object",
                        "properties": {
                            "param1": {"type": "string", "description": "A very long parameter description that adds to the size"},
                            "param2": {"type": "number", "description": "Another long description for testing purposes"}
                        }
                    }),
                )
            })
            .collect();

        let original_size = estimate_tools_size(&tools);
        assert!(
            original_size > 20 * 1024,
            "测试数据应超过阈值"
        );

        let result = compress_tools_if_needed(&tools, &CompressionConfig::default());
        let compressed_size = estimate_tools_size(&result);
        assert!(
            compressed_size < original_size,
            "压缩后应更小: {} < {}",
            compressed_size,
            original_size
        );
    }

    #[test]
    fn test_simplify_schema_removes_descriptions() {
        let tool = make_tool(
            "test",
            "desc",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "The file path to read"
                    }
                },
                "required": ["path"]
            }),
        );

        let simplified = simplify_schema(&tool);
        let props = simplified
            .tool_specification
            .input_schema
            .json
            .get("properties")
            .unwrap();
        let path_prop = props.get("path").unwrap();

        // description 应被移除
        assert!(path_prop.get("description").is_none());
        // type 应保留
        assert_eq!(path_prop.get("type").unwrap(), "string");
    }
}
