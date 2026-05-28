//! 模拟结构化输出
//!
//! 通过系统提示词注入 JSON Schema 约束，并对响应进行 markdown fence 剥离。

use pulldown_cmark::{CodeBlockKind, Event, Parser, Tag, TagEnd};

/// 生成结构化输出的系统提示词注入内容
pub fn generate_schema_instruction(schema: &serde_json::Value) -> String {
    let schema_str = serde_json::to_string_pretty(schema).unwrap_or_else(|_| schema.to_string());
    format!(
        "You MUST respond with ONLY valid JSON (no markdown code fences, no explanation, no additional text) that conforms to the following JSON Schema:\n\n{}\n\nOutput nothing but the raw JSON object or array. Do not wrap it in code fences or add any other text.",
        schema_str
    )
}

/// 从可能包含 markdown 代码块的文本中提取 JSON 内容
///
/// 优先提取 `json` 语言标记的代码块，其次提取任意代码块。
/// 如果没有代码块，返回原始文本（去除首尾空白）。
pub fn extract_json_from_response(text: &str) -> String {
    let trimmed = text.trim();

    // 尝试用 pulldown-cmark 解析提取代码块
    let parser = Parser::new(trimmed);
    let mut in_code_block = false;
    let mut code_lang = String::new();
    let mut code_content = String::new();
    let mut first_any_block: Option<String> = None;
    let mut json_block: Option<String> = None;

    for event in parser {
        match event {
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(lang))) => {
                in_code_block = true;
                code_lang = lang.to_string();
                code_content.clear();
            }
            Event::Text(text) if in_code_block => {
                code_content.push_str(&text);
            }
            Event::End(TagEnd::CodeBlock) if in_code_block => {
                in_code_block = false;
                let content = code_content.trim().to_string();
                if code_lang == "json" || code_lang == "JSON" {
                    json_block = Some(content);
                } else if first_any_block.is_none() && !content.is_empty() {
                    first_any_block = Some(content);
                }
            }
            _ => {}
        }
    }

    // 优先返回 json 标记的代码块
    if let Some(json) = json_block {
        return json;
    }

    // 其次返回第一个代码块
    if let Some(block) = first_any_block {
        return block;
    }

    // 没有代码块，返回原始文本
    trimmed.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_json_from_plain_json() {
        let input = r#"{"name": "test", "value": 42}"#;
        assert_eq!(extract_json_from_response(input), input);
    }

    #[test]
    fn test_extract_json_from_fenced_block() {
        let input = "```json\n{\"name\": \"test\"}\n```";
        assert_eq!(extract_json_from_response(input), "{\"name\": \"test\"}");
    }

    #[test]
    fn test_extract_json_with_surrounding_text() {
        let input = "Here is the result:\n\n```json\n{\"foo\": \"bar\"}\n```\n\nHope that helps!";
        assert_eq!(extract_json_from_response(input), "{\"foo\": \"bar\"}");
    }

    #[test]
    fn test_extract_json_unlabeled_fence() {
        let input = "```\n{\"key\": \"value\"}\n```";
        assert_eq!(extract_json_from_response(input), "{\"key\": \"value\"}");
    }

    #[test]
    fn test_extract_json_no_fence_returns_trimmed() {
        let input = "  {\"raw\": true}  ";
        assert_eq!(extract_json_from_response(input), "{\"raw\": true}");
    }

    #[test]
    fn test_generate_schema_instruction() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {"name": {"type": "string"}},
            "required": ["name"]
        });
        let instruction = generate_schema_instruction(&schema);
        assert!(instruction.contains("JSON Schema"));
        assert!(instruction.contains("\"name\""));
        assert!(instruction.contains("no markdown code fences"));
    }
}
