use std::fmt;
use std::io::Cursor;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use oxidize_pdf::parser::{PdfDocument, PdfReader};

/// PDF 文本提取错误
#[derive(Debug)]
pub enum PdfError {
    Base64Decode(base64::DecodeError),
    Parse(String),
    Extract(String),
}

impl fmt::Display for PdfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PdfError::Base64Decode(e) => write!(f, "Base64 解码失败: {e}"),
            PdfError::Parse(e) => write!(f, "PDF 解析失败: {e}"),
            PdfError::Extract(e) => write!(f, "文本提取失败: {e}"),
        }
    }
}

impl From<base64::DecodeError> for PdfError {
    fn from(e: base64::DecodeError) -> Self {
        PdfError::Base64Decode(e)
    }
}

/// 从 base64 编码的 PDF 数据中提取文本
///
/// 返回所有页面的文本内容，页间以换行分隔
pub fn extract_text_from_base64(data: &str) -> Result<String, PdfError> {
    let bytes = STANDARD.decode(data)?;
    extract_text_from_bytes(bytes)
}

/// 从 PDF 字节数据中提取文本
pub fn extract_text_from_bytes(data: Vec<u8>) -> Result<String, PdfError> {
    let cursor = Cursor::new(data);
    let reader = PdfReader::new(cursor).map_err(|e| PdfError::Parse(e.to_string()))?;
    let document = PdfDocument::new(reader);

    let pages = document
        .extract_text()
        .map_err(|e| PdfError::Extract(e.to_string()))?;

    let text = pages
        .iter()
        .map(|page| page.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");

    Ok(text)
}
