//! Parse quality layer — contract, QA gates and degraded-mode marker for
//! the builtin parse path (kb-parser-engines-design §3, E0).
//!
//! Parsing is where RAG quality is born: parse errors are silent (nothing
//! errors, retrieval just gets worse), permanent (re-indexing can't recover
//! content lost at parse time) and compounding (bad markdown → bad chunks →
//! bad vectors). The gates below make the two catastrophic cases LOUD
//! (empty / mojibake content never enters the index) and the partial case
//! VISIBLE (scanned-page degradation is a first-class marker on the doc
//! row, not a buried log line).

use crate::errors::app_error::{AppError, AppResult};
use crate::kb::parser::ParseOutcome;

/// Placeholder line inserted by `extract_pdf_skip_ocr` for scanned pages —
/// excluded from the effective-content measure (a doc of pure placeholders
/// carries zero retrievable content).
fn is_placeholder_line(line: &str) -> bool {
    let t = line.trim();
    t.starts_with("> [第") && t.contains("扫描件") && t.ends_with("]")
}

/// Chars of real content: total non-whitespace minus placeholder lines.
pub fn effective_chars(markdown: &str) -> usize {
    markdown
        .lines()
        .filter(|l| !is_placeholder_line(l))
        .map(|l| l.chars().filter(|c| !c.is_whitespace()).count())
        .sum()
}

/// Below this the document carries no retrievable content.
const EMPTY_EFFECTIVE_CHARS: usize = 16;
/// Replacement/private-use char share of the effective content above which
/// the parse is treated as encoding-corrupted garbage.
const MOJIBAKE_RATIO: f64 = 0.05;

fn mojibake_chars(markdown: &str) -> (usize, usize) {
    let mut bad = 0usize;
    let mut total = 0usize;
    for c in markdown.chars() {
        if c.is_whitespace() {
            continue;
        }
        total += 1;
        let cp = c as u32;
        if cp == 0xFFFD || (0xE000..=0xF8FF).contains(&cp) {
            bad += 1;
        }
    }
    (bad, total)
}

/// Gate verdict: `Ok(warnings)` passes (warnings recorded on the run/doc),
/// `Err` fails the document loudly — never index empty or corrupted content.
pub fn gate(doc: &ParseOutcome) -> AppResult<Vec<String>> {
    let mut warnings = Vec::new();
    let effective = effective_chars(&doc.markdown);
    if effective < EMPTY_EFFECTIVE_CHARS {
        let hint = if !doc.scanned_pages.is_empty() {
            "；文档以扫描页为主，请配置解析引擎（docreader）后重新解析"
        } else {
            ""
        };
        return Err(AppError::BadRequest(format!(
            "解析产物为空（有效文本 {effective} 字符）——拒绝入库{hint}"
        )));
    }
    let (bad, total) = mojibake_chars(&doc.markdown);
    if total > 0 && bad as f64 / total as f64 > MOJIBAKE_RATIO {
        return Err(AppError::BadRequest(format!(
            "疑似编码损坏（乱码字符占比 {:.0}%）——拒绝入库",
            bad as f64 / total as f64 * 100.0
        )));
    }
    if let Some(total_pages) = doc.pages
        && !doc.scanned_pages.is_empty()
    {
        warnings.push(format!(
            "扫描页跳过 {}/{}：{:?}",
            doc.scanned_pages.len(),
            total_pages,
            doc.scanned_pages
        ));
        let text_pages = total_pages.saturating_sub(doc.scanned_pages.len() as u32);
        if total_pages > 0 && (text_pages as f64 / total_pages as f64) < 0.5 {
            warnings.push(format!(
                "页覆盖率过低（文本页 {text_pages}/{total_pages}）——建议配置解析引擎"
            ));
        }
    }
    Ok(warnings)
}

/// Degraded-mode marker for the doc row (`kb_documents.parse_degraded`
/// JSON; `None` = fully parsed). Admin surfaces it as a "部分解析" badge.
pub fn degraded_marker(doc: &ParseOutcome) -> Option<String> {
    if doc.scanned_pages.is_empty() {
        return None;
    }
    Some(
        serde_json::json!({
            "engine": doc.engine,
            "scanned_pages": doc.scanned_pages,
        })
        .to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(md: &str) -> ParseOutcome {
        ParseOutcome {
            markdown: md.into(),
            engine: "builtin".into(),
            ..Default::default()
        }
    }

    #[test]
    fn empty_content_is_rejected() {
        let err = gate(&doc("   \n\n  ")).unwrap_err();
        assert!(err.to_string().contains("解析产物为空"));
    }

    #[test]
    fn placeholder_only_pdf_is_rejected_with_engine_hint() {
        let mut d = doc(
            "> [第 1 页为扫描件（scanned page），需要 OCR，已跳过]\n\n> [第 2 页为扫描件（scanned page），需要 OCR，已跳过]",
        );
        d.pages = Some(2);
        d.scanned_pages = vec![0, 1];
        let err = gate(&d).unwrap_err();
        assert!(err.to_string().contains("解析引擎"), "hint engine: {err}");
    }

    #[test]
    fn partial_scan_passes_with_warnings() {
        let mut d = doc(
            "# 标题\n\n正文内容足够多以通过空文门禁这是一段真实的文档内容。\n\n> [第 3 页为扫描件（scanned page），需要 OCR，已跳过]",
        );
        d.pages = Some(3);
        d.scanned_pages = vec![2];
        let warnings = gate(&d).unwrap();
        assert!(warnings.iter().any(|w| w.contains("扫描页跳过 1/3")));
        assert!(degraded_marker(&d).is_some());
    }

    #[test]
    fn mojibake_is_rejected() {
        let garbage: String = format!("{}{}", "正常开头\u{FFFD}".repeat(20), "\u{E000}".repeat(20));
        let err = gate(&doc(&garbage)).unwrap_err();
        assert!(err.to_string().contains("疑似编码损坏"));
    }

    #[test]
    fn clean_doc_passes_silently() {
        let d = doc("# 安装指南\n\n通过 cargo 安装 raisfast 服务端，支持三种数据库后端。");
        assert!(gate(&d).unwrap().is_empty());
        assert!(degraded_marker(&d).is_none());
    }
}

/// Page anchor line emitted by the (patched) docreader PDF parser.
pub const PAGE_MARK: &str = "<!-- page:";

/// Byte offsets of every page anchor, in order: `(page, offset)`.
pub fn page_marks(markdown: &str) -> Vec<(u32, usize)> {
    let mut out = Vec::new();
    let mut off = 0usize;
    for line in markdown.split_inclusive('\n') {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix(PAGE_MARK)
            && let Some(num) = rest.strip_prefix_end()
            && let Ok(page) = num.parse::<u32>()
        {
            out.push((page, off));
        }
        off += line.len();
    }
    out
}

trait StripPrefixEnd {
    fn strip_prefix_end(&self) -> Option<&str>;
}
impl StripPrefixEnd for str {
    fn strip_prefix_end(&self) -> Option<&str> {
        self.strip_suffix("-->").map(|n| n.trim_end())
    }
}

/// Page containing `offset` (1-based anchor semantics: the last anchor at
/// or before the offset). `None` when the markdown carries no anchors.
pub fn page_of(marks: &[(u32, usize)], offset: usize) -> Option<u32> {
    if marks.is_empty() {
        return None;
    }
    Some(
        marks
            .iter()
            .rev()
            .find(|(_, mo)| *mo <= offset)
            .map(|(p, _)| *p)
            .unwrap_or(1),
    )
}

/// Blank out page-anchor lines while PRESERVING total byte length —
/// downstream byte offsets (chunk.byte_start, image refs) stay aligned
/// with the original markdown.
pub fn blank_page_marks(markdown: &str) -> String {
    let mut out = String::with_capacity(markdown.len());
    for line in markdown.split_inclusive('\n') {
        if line.trim().starts_with(PAGE_MARK) {
            out.push_str(&" ".repeat(line.len()));
        } else {
            out.push_str(line);
        }
    }
    out
}

/// Remove page-anchor lines from chunk content (anchors ride the markdown
/// for offset mapping; they must not leak into retrieval text).
pub fn strip_page_marks(content: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    let mut prev_empty = false;
    for l in content.lines() {
        if l.trim().starts_with(PAGE_MARK) {
            continue;
        }
        let empty = l.trim().is_empty();
        if empty && prev_empty {
            continue; // collapse the blank pair the anchor line leaves behind
        }
        prev_empty = empty;
        if empty && out.is_empty() {
            continue;
        }
        out.push(l);
    }
    while out.last().is_some_and(|l| l.trim().is_empty()) {
        out.pop();
    }
    out.join("\n")
}

/// Page number parsed out of a docreader image filename
/// (`{base}_p3_img1.jpg` / `{base}_page_5.jpg`).
pub fn page_from_filename(name: &str) -> Option<i64> {
    for marker in ["_page_", "_p"] {
        let mut start = 0usize;
        while let Some(pos) = name[start..].find(marker) {
            let after = start + pos + marker.len();
            let digits: String = name[after..]
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect();
            if let Ok(n) = digits.parse::<i64>() {
                return Some(n);
            }
            start = start + pos + 1;
        }
    }
    None
}

#[cfg(test)]
mod page_tests {
    use super::*;

    #[test]
    fn marks_map_offsets_to_pages() {
        let md = "intro\n\n<!-- page:1 -->\n\ntext a\n\n<!-- page:2 -->\n\ntext b";
        let marks = page_marks(md);
        assert_eq!(marks.len(), 2);
        assert_eq!(page_of(&marks, 0), Some(1));
        assert_eq!(page_of(&marks, md.find("text a").unwrap()), Some(1));
        assert_eq!(page_of(&marks, md.find("text b").unwrap()), Some(2));
    }

    #[test]
    fn no_marks_yields_none() {
        assert!(page_of(&page_marks("plain"), 0).is_none());
    }

    #[test]
    fn marks_stripped_from_content() {
        assert_eq!(strip_page_marks("a\n\n<!-- page:2 -->\n\nb"), "a\n\nb");
    }

    #[test]
    fn filename_page_parsed() {
        assert_eq!(
            page_from_filename("01a0af2c-391b-7db2-87ad-7323122616d8_p3_img1.jpg"),
            Some(3)
        );
        assert_eq!(page_from_filename("doc_page_5.jpg"), Some(5));
        assert_eq!(page_from_filename("plain.png"), None);
    }
}
