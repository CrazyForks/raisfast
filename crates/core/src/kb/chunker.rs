//! Chunker — thin wrapper over `text-splitter` + `comrak`
//! (kb-technical-design §3, dependency audit §15).
//!
//! Splitting kernel: `MarkdownSplitter` (CommonMark+GFM AST, heading levels
//! as semantic boundaries with automatic level fallback) [抄EXT:text-splitter].
//! Breadcrumbs: comrak AST heading walk, WK `header_tracker` semantics
//! [抄WK:职责 + 抄EXT:comrak 实现]. Parent-child: two-pass split
//! (parent 4096 → child 384, children embedded, parents fed to the LLM)
//! [抄WK:docs/CHUNKING.md parent-child 语义]. Declared deviation: no chunk
//! overlap (text-splitter lacks it; parent-child expansion compensates).

use comrak::nodes::NodeValue;
use comrak::{Arena, Options, parse_document};

/// Default chunking parameters, WK baseline values [抄WK:CHUNKING.md].
pub const DEFAULT_PARENT_SIZE: usize = 4096;
pub const DEFAULT_CHILD_SIZE: usize = 384;

/// Chunking knobs (per-KB overrides land with KB config in M2 admin APIs;
/// v1 uses the WK defaults).
#[derive(Debug, Clone, Copy)]
pub struct ChunkerConfig {
    pub parent_size: usize,
    pub child_size: usize,
}

impl Default for ChunkerConfig {
    fn default() -> Self {
        Self {
            parent_size: DEFAULT_PARENT_SIZE,
            child_size: DEFAULT_CHILD_SIZE,
        }
    }
}

/// A produced chunk with source span and heading breadcrumb.
#[derive(Debug, Clone, PartialEq)]
pub struct Chunk {
    pub content: String,
    /// Heading path like `# Top > ## Section` (empty when no heading).
    pub breadcrumb: String,
    pub byte_start: usize,
    pub byte_end: usize,
    /// `None` for parent chunks; `Some(parent_index)` for children.
    pub parent: Option<usize>,
}

/// Split markdown into parent/child chunks.
///
/// Returns a flat list: parent chunks first, each followed by its children
/// (children reference parents by index).
pub fn chunk_markdown(markdown: &str, cfg: &ChunkerConfig) -> Vec<Chunk> {
    let headings = collect_heading_ranges(markdown);

    let parent_splitter = text_splitter::MarkdownSplitter::new(cfg.parent_size);
    let child_splitter = text_splitter::MarkdownSplitter::new(cfg.child_size);

    let mut out: Vec<Chunk> = Vec::new();
    for (parent_off, parent_text) in parent_splitter.chunk_indices(markdown) {
        let parent_start = parent_off;
        let parent_end = parent_off + parent_text.len();
        let parent_index = out.len();
        out.push(Chunk {
            content: parent_text.to_string(),
            breadcrumb: breadcrumb_for(&headings, parent_start),
            byte_start: parent_start,
            byte_end: parent_end,
            parent: None,
        });

        if parent_text.len() <= cfg.child_size {
            continue;
        }
        for (child_off, child_text) in child_splitter.chunk_indices(parent_text) {
            let abs = parent_start + child_off;
            out.push(Chunk {
                content: child_text.to_string(),
                breadcrumb: breadcrumb_for(&headings, abs),
                byte_start: abs,
                byte_end: abs + child_text.len(),
                parent: Some(parent_index),
            });
        }
    }
    out
}

/// Heading (level, byte_start, title) triples derived from the comrak AST —
/// one parse feeds every breadcrumb lookup.
fn collect_heading_ranges(markdown: &str) -> Vec<(u8, usize, String)> {
    let arena = Arena::new();
    let mut options = Options::default();
    options.extension.table = true;
    options.extension.strikethrough = true;
    options.extension.tasklist = true;
    let root = parse_document(&arena, markdown, &options);

    // line number → byte offset table (comrak sourcepos is line/col 1-based).
    let mut line_starts = Vec::with_capacity(64);
    line_starts.push(0usize);
    for (off, b) in markdown.bytes().enumerate() {
        if b == b'\n' {
            line_starts.push(off + 1);
        }
    }

    let mut headings = Vec::new();
    fn walk<'a>(
        node: &'a comrak::nodes::AstNode<'a>,
        line_starts: &[usize],
        markdown: &str,
        headings: &mut Vec<(u8, usize, String)>,
    ) {
        if let NodeValue::Heading(h) = &node.data.borrow().value {
            let pos = node.data.borrow().sourcepos;
            let start = line_starts
                .get(pos.start.line.saturating_sub(1))
                .copied()
                .unwrap_or(0)
                + pos.start.column.saturating_sub(1);
            if start <= markdown.len() {
                headings.push((h.level, start, heading_text(node)));
            }
        }
        for child in node.children() {
            walk(child, line_starts, markdown, headings);
        }
    }
    fn heading_text<'a>(node: &'a comrak::nodes::AstNode<'a>) -> String {
        let mut text = String::new();
        fn collect<'a>(n: &'a comrak::nodes::AstNode<'a>, out: &mut String) {
            if let NodeValue::Text(t) = &n.data.borrow().value {
                out.push_str(t);
            }
            for c in n.children() {
                collect(c, out);
            }
        }
        collect(node, &mut text);
        text.trim().to_string()
    }
    walk(root, &line_starts, markdown, &mut headings);
    headings.sort_by_key(|&(_, start, _)| start);
    headings
}

/// Heading path at a byte position, like `# Top > ## Section` — stack
/// semantics: a deeper-or-equal level replaces shallower ones
/// [抄WK:header_tracker.go 职责].
fn breadcrumb_for(headings: &[(u8, usize, String)], at: usize) -> String {
    let mut stack: Vec<(u8, String)> = Vec::new();
    for &(level, start, ref title) in headings {
        if start > at {
            break;
        }
        while stack.last().is_some_and(|(l, _)| *l >= level) {
            stack.pop();
        }
        stack.push((level, title.clone()));
    }
    stack
        .iter()
        .map(|(l, t)| {
            if t.is_empty() {
                "#".repeat(*l as usize)
            } else {
                format!("{} {}", "#".repeat(*l as usize), t)
            }
        })
        .collect::<Vec<_>>()
        .join(" > ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_by_headings_and_sizes() {
        let mut md = "# A\n\nintro text\n\n## B\n\n".to_string();
        md.push_str(&"word ".repeat(200)); // > child size, forces parent+child
        let chunks = chunk_markdown(&md, &ChunkerConfig::default());
        assert!(
            chunks.len() >= 3,
            "expected parents + children, got {}",
            chunks.len()
        );
        let parents = chunks.iter().filter(|c| c.parent.is_none()).count();
        let children = chunks.iter().filter(|c| c.parent.is_some()).count();
        assert!(parents >= 1);
        assert!(children >= 1);
        // every child points at a valid parent index
        for c in &chunks {
            if let Some(p) = c.parent {
                assert!(c.parent.is_some_and(|_| p < chunks.len()));
                assert!(chunks[p].parent.is_none(), "parent points at parent");
            }
        }
        // spans stay inside the source and are ordered
        for w in chunks.windows(2) {
            assert!(w[0].byte_start <= w[1].byte_start);
        }
        assert!(chunks.last().is_some_and(|c| c.byte_end <= md.len()));
    }

    #[test]
    fn short_doc_single_parent_no_children() {
        let md = "# Title\n\nshort body\n";
        let chunks = chunk_markdown(md, &ChunkerConfig::default());
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].parent, None);
        assert!(chunks[0].content.contains("short body"));
    }

    #[test]
    fn breadcrumb_reflects_heading_depth() {
        // Small parent size forces splits at heading boundaries; the chunk
        // that *starts* inside the Sub section must carry the full path
        // (breadcrumb = heading path at chunk start, WK header_tracker
        // semantics — a chunk spanning sections reports its start path).
        let md = format!(
            "# Top\n\n{}\n\n## Sub\n\n{}",
            "alpha ".repeat(80),
            "beta ".repeat(80)
        );
        let cfg = ChunkerConfig {
            parent_size: 200,
            child_size: 200,
        };
        let chunks = chunk_markdown(&md, &cfg);
        let sub = chunks
            .iter()
            .find(|c| c.content.trim_start().starts_with("beta"))
            .unwrap_or_else(|| panic!("sub-starting chunk missing: {chunks:?}"));
        assert_eq!(sub.breadcrumb, "# Top > ## Sub");
        let top = chunks
            .iter()
            .find(|c| c.content.trim_start().starts_with("alpha"))
            .unwrap_or_else(|| panic!("top-starting chunk missing"));
        assert_eq!(top.breadcrumb, "# Top");
    }

    #[test]
    fn roundtrip_spans_slice_source() {
        let mut md = "# H\n\n".to_string();
        md.push_str(&"数据 ".repeat(300));
        let chunks = chunk_markdown(&md, &ChunkerConfig::default());
        for c in &chunks {
            let slice = &md[c.byte_start..c.byte_end];
            assert_eq!(slice, c.content, "span must slice the source exactly");
        }
    }
}
