use std::fmt;
use std::io::Cursor;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use oxidize_pdf::parser::{PdfDocument, PdfReader};

const MAX_RAW_FALLBACK_PDF_BYTES: usize = 16 * 1024;

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
    match extract_text_with_oxidize(&data) {
        Ok(text) => Ok(text),
        Err(primary_err) if data.len() <= MAX_RAW_FALLBACK_PDF_BYTES => {
            extract_text_from_uncompressed_streams(&data).ok_or(primary_err)
        }
        Err(primary_err) => Err(primary_err),
    }
}

fn extract_text_with_oxidize(data: &[u8]) -> Result<String, PdfError> {
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

fn extract_text_from_uncompressed_streams(data: &[u8]) -> Option<String> {
    let mut parts = Vec::new();
    let mut offset = 0;

    while let Some(stream_rel) = find_subslice(&data[offset..], b"stream") {
        let stream_marker_start = offset + stream_rel;
        let mut content_start = stream_marker_start + b"stream".len();

        if data.get(content_start..content_start + 2) == Some(b"\r\n") {
            content_start += 2;
        } else if matches!(data.get(content_start), Some(b'\n' | b'\r')) {
            content_start += 1;
        }

        let Some(end_rel) = find_subslice(&data[content_start..], b"endstream") else {
            break;
        };
        let content_end = content_start + end_rel;

        if let Some(text) = extract_text_from_content_stream(&data[content_start..content_end]) {
            parts.push(text);
        }

        offset = content_end + b"endstream".len();
    }

    let text = parts.join("\n");
    (!text.trim().is_empty()).then_some(text)
}

fn extract_text_from_content_stream(stream: &[u8]) -> Option<String> {
    let mut text_objects = Vec::new();
    let mut offset = 0;

    while let Some(bt_rel) = find_pdf_token(&stream[offset..], b"BT") {
        let content_start = offset + bt_rel + b"BT".len();
        let Some(et_rel) = find_pdf_token(&stream[content_start..], b"ET") else {
            break;
        };
        let content_end = content_start + et_rel;
        let text = extract_literal_strings(&stream[content_start..content_end]).join("");
        if !text.trim().is_empty() {
            text_objects.push(text);
        }
        offset = content_end + b"ET".len();
    }

    if text_objects.is_empty() {
        let text = extract_literal_strings(stream).join("");
        (!text.trim().is_empty()).then_some(text)
    } else {
        Some(text_objects.join("\n"))
    }
}

fn extract_literal_strings(data: &[u8]) -> Vec<String> {
    let mut strings = Vec::new();
    let mut offset = 0;

    while offset < data.len() {
        if data[offset] == b'(' {
            if let Some((value, next_offset)) = parse_literal_string(data, offset) {
                strings.push(value);
                offset = next_offset;
                continue;
            }
        }
        offset += 1;
    }

    strings
}

fn parse_literal_string(data: &[u8], start: usize) -> Option<(String, usize)> {
    let mut out = Vec::new();
    let mut depth = 1usize;
    let mut offset = start + 1;

    while offset < data.len() {
        match data[offset] {
            b'\\' => {
                offset += 1;
                if offset >= data.len() {
                    break;
                }
                match data[offset] {
                    b'n' => out.push(b'\n'),
                    b'r' => out.push(b'\r'),
                    b't' => out.push(b'\t'),
                    b'b' => out.push(0x08),
                    b'f' => out.push(0x0c),
                    b'\n' => {}
                    b'\r' => {
                        if data.get(offset + 1) == Some(&b'\n') {
                            offset += 1;
                        }
                    }
                    b'0'..=b'7' => {
                        let mut value = data[offset] - b'0';
                        for _ in 0..2 {
                            if let Some(next @ b'0'..=b'7') = data.get(offset + 1).copied() {
                                offset += 1;
                                value = value.saturating_mul(8).saturating_add(next - b'0');
                            } else {
                                break;
                            }
                        }
                        out.push(value);
                    }
                    other => out.push(other),
                }
            }
            b'(' => {
                depth += 1;
                out.push(b'(');
            }
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some((String::from_utf8_lossy(&out).into_owned(), offset + 1));
                }
                out.push(b')');
            }
            other => out.push(other),
        }
        offset += 1;
    }

    None
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn find_pdf_token(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .enumerate()
        .find_map(|(idx, window)| {
            (window == needle && is_token_boundary(haystack, idx, needle.len())).then_some(idx)
        })
}

fn is_token_boundary(data: &[u8], start: usize, len: usize) -> bool {
    let before = start.checked_sub(1).and_then(|idx| data.get(idx).copied());
    let after = data.get(start + len).copied();
    before.is_none_or(is_pdf_delimiter) && after.is_none_or(is_pdf_delimiter)
}

fn is_pdf_delimiter(byte: u8) -> bool {
    byte.is_ascii_whitespace()
        || matches!(byte, b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'/' | b'%')
}

#[cfg(test)]
mod tests {
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;

    use super::{PdfError, extract_text_from_base64, extract_text_from_bytes};

    const MALFORMED_XREF_PDF_HVOYOWSE: &str = "JVBERi0xLjQKMSAwIG9iago8PCAvVHlwZSAvQ2F0YWxvZyAvUGFnZXMgMiAwIFIgPj4KZW5kb2JqCjIgMCBvYmoKPDwgL1R5cGUgL1BhZ2VzIC9LaWRzIFszIDAgUl0gL0NvdW50IDEgPj4KZW5kb2JqCjMgMCBvYmoKPDwgL1R5cGUgL1BhZ2UgL1BhcmVudCAyIDAgUiAvTWVkaWFCb3ggWzAgMCAxNTAgNTBdIC9SZXNvdXJjZXMgPDwgL0ZvbnQgPDwgL0YxIDUgMCBSID4+ID4+IC9Db250ZW50cyA0IDAgUiA+PgplbmRvYmoKNCAwIG9iago8PCAvTGVuZ3RoIDM4ID4+CnN0cmVhbQpCVCAvRjEgMTQgVGYgMTAgMjAgVGQgKGh2b3lvd3NlKSBUaiBFVAplbmRzdHJlYW0KZW5kb2JqCjUgMCBvYmoKPDwgL1R5cGUgL0ZvbnQgL1N1YnR5cGUgL1R5cGUxIC9CYXNlRm9udCAvSGVsdmV0aWNhID4+CmVuZG9iagp4cmVmCjAgNgowMDAwMDAwMDAwIDY1NTM1IGYgCnRyYWlsZXIKPDwgL1NpemUgNiAvUm9vdCAxIDAgUiA+PgpzdGFydHhyZWYKMAolJUVPRg==";
    const MALFORMED_XREF_PDF_HVOYVJIM: &str = "JVBERi0xLjQKMSAwIG9iago8PCAvVHlwZSAvQ2F0YWxvZyAvUGFnZXMgMiAwIFIgPj4KZW5kb2JqCjIgMCBvYmoKPDwgL1R5cGUgL1BhZ2VzIC9LaWRzIFszIDAgUl0gL0NvdW50IDEgPj4KZW5kb2JqCjMgMCBvYmoKPDwgL1R5cGUgL1BhZ2UgL1BhcmVudCAyIDAgUiAvTWVkaWFCb3ggWzAgMCAxNTAgNTBdIC9SZXNvdXJjZXMgPDwgL0ZvbnQgPDwgL0YxIDUgMCBSID4+ID4+IC9Db250ZW50cyA0IDAgUiA+PgplbmRvYmoKNCAwIG9iago8PCAvTGVuZ3RoIDM4ID4+CnN0cmVhbQpCVCAvRjEgMTQgVGYgMTAgMjAgVGQgKGh2b3l2amltKSBUaiBFVAplbmRzdHJlYW0KZW5kb2JqCjUgMCBvYmoKPDwgL1R5cGUgL0ZvbnQgL1N1YnR5cGUgL1R5cGUxIC9CYXNlRm9udCAvSGVsdmV0aWNhID4+CmVuZG9iagp4cmVmCjAgNgowMDAwMDAwMDAwIDY1NTM1IGYgCnRyYWlsZXIKPDwgL1NpemUgNiAvUm9vdCAxIDAgUiA+PgpzdGFydHhyZWYKMAolJUVPRg==";

    #[test]
    fn extracts_text_from_malformed_xref_pdf_hvoyowse() {
        let text = extract_text_from_base64(MALFORMED_XREF_PDF_HVOYOWSE)
            .expect("malformed xref PDF should be parsed by fallback extractor");

        assert_eq!(text.trim(), "hvoyowse");
    }

    #[test]
    fn extracts_text_from_malformed_xref_pdf_hvoyvjim() {
        let text = extract_text_from_base64(MALFORMED_XREF_PDF_HVOYVJIM)
            .expect("malformed xref PDF should be parsed by fallback extractor");

        assert_eq!(text.trim(), "hvoyvjim");
    }

    #[test]
    fn does_not_run_fallback_for_large_malformed_pdf() {
        let mut bytes = STANDARD.decode(MALFORMED_XREF_PDF_HVOYOWSE).unwrap();
        bytes.resize(16 * 1024 + 1, b' ');

        let err = extract_text_from_bytes(bytes)
            .expect_err("large malformed PDFs should not use raw stream fallback");

        assert!(matches!(err, PdfError::Parse(_)));
    }
}
