//! SKILL.md document parsing/serialization — ported from zeroclaw
//! (`zeroclaw-runtime/src/skills/document.rs`, MIT/Apache-2.0, reference C-S1).
//!
//! Behavior preserved verbatim (split-frontmatter, flat `key: value` parser with
//! YAML block scalars, tolerant unknown keys, round-trip writers). M5-A port
//! skips typed `slash_options` parsing (they are stored-and-ignored per
//! skills.md §12-E) but still skips the nested `slash_options:` block so its
//! indented lines never hijack flat keys such as `tags:`.

use std::fmt::Write as _;

/// Frontmatter of a `SKILL.md` (zeroclaw field set; unknown keys tolerated).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SkillFrontmatter {
    pub name: String,
    pub description: String,
    pub license: Option<String>,
    pub author: Option<String>,
    pub version: Option<String>,
    pub category: Option<String>,
    pub tags: Vec<String>,
    /// Keep instructions inlined even in Compact mode.
    pub always: bool,
}

/// A parsed `SKILL.md`.
#[derive(Debug, Clone, PartialEq)]
pub struct SkillDocument {
    pub frontmatter: SkillFrontmatter,
    pub body: String,
}

#[derive(Debug, thiserror::Error)]
pub enum SkillDocError {
    #[error("SKILL.md is missing the leading `---` frontmatter delimiter")]
    MissingFrontmatter,
    #[error("SKILL.md frontmatter is missing required field `{0}`")]
    MissingRequiredField(&'static str),
    #[error("io error reading skill file: {0}")]
    Io(String),
}

impl SkillDocument {
    /// Parse a SKILL.md. Mirrors zeroclaw `document.rs::SkillDocument::parse`.
    pub fn parse(content: &str) -> Result<Self, SkillDocError> {
        let (frontmatter_src, body) =
            split_frontmatter(content).ok_or(SkillDocError::MissingFrontmatter)?;
        let frontmatter = parse_frontmatter(&frontmatter_src)?;
        let body = body.strip_prefix('\n').unwrap_or(body.as_str());
        Ok(Self {
            frontmatter,
            body: body.to_string(),
        })
    }

    /// Serialize back to SKILL.md text (byte-stable for tagless/always-less skills).
    pub fn serialize(&self) -> String {
        let mut out = String::with_capacity(self.body.len() + 256);
        out.push_str("---\n");
        write_field(&mut out, "name", &self.frontmatter.name);
        write_block_scalar(&mut out, "description", &self.frontmatter.description);
        write_optional(&mut out, "license", self.frontmatter.license.as_deref());
        write_optional(&mut out, "author", self.frontmatter.author.as_deref());
        write_optional(&mut out, "version", self.frontmatter.version.as_deref());
        write_optional(&mut out, "category", self.frontmatter.category.as_deref());
        write_tags(&mut out, &self.frontmatter.tags);
        write_bool(&mut out, "always", self.frontmatter.always);
        out.push_str("---\n");
        if !self.body.is_empty() {
            if !self.body.starts_with('\n') {
                out.push('\n');
            }
            out.push_str(&self.body);
            if !self.body.ends_with('\n') {
                out.push('\n');
            }
        }
        out
    }
}

// ── ported helpers (zeroclaw `document.rs`) ────────────────────────────────

/// Split `---\n...\n---\n` from the body. Mirrors zeroclaw `split_frontmatter`.
pub fn split_frontmatter(content: &str) -> Option<(String, String)> {
    let normalized = content.replace("\r\n", "\n");
    let rest = normalized.strip_prefix("---\n")?;
    if let Some(idx) = rest.find("\n---\n") {
        return Some((rest[..idx].to_string(), rest[idx + 5..].to_string()));
    }
    if let Some(frontmatter) = rest.strip_suffix("\n---") {
        return Some((frontmatter.to_string(), String::new()));
    }
    None
}

fn parse_frontmatter(src: &str) -> Result<SkillFrontmatter, SkillDocError> {
    let mut fm = SkillFrontmatter::default();
    let mut multiline: Option<(String, Vec<String>)> = None;
    let mut collecting_tags = false;
    // Carve out the nested `slash_options:` block (ported from zeroclaw) so its
    // indented lines never misread as flat keys. Options themselves are not
    // parsed in M5-A (stored-and-ignored).
    let slash_block = locate_slash_options_block(src);

    let flush = |fm: &mut SkillFrontmatter, key: &str, parts: &[String]| {
        let val = parts.join(" ");
        let val = val.trim();
        if val.is_empty() {
            return;
        }
        assign(fm, key, val);
    };

    for (idx, line) in src.lines().enumerate() {
        if let Some((start, end)) = slash_block
            && idx >= start
            && idx < end
        {
            continue;
        }
        if let Some((ref key, ref mut parts)) = multiline {
            if line.starts_with(' ') || line.starts_with('\t') || line.trim().is_empty() {
                parts.push(line.trim().to_string());
                continue;
            }
            let (key_owned, parts_owned) = (key.clone(), std::mem::take(parts));
            flush(&mut fm, &key_owned, &parts_owned);
            multiline = None;
        }
        if collecting_tags {
            let trimmed = line.trim();
            if let Some(item) = trimmed.strip_prefix("- ") {
                let tag = item.trim().trim_matches('"').trim_matches('\'');
                if !tag.is_empty() {
                    fm.tags.push(tag.to_string());
                }
                continue;
            }
            collecting_tags = false;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim().trim_matches('"').trim_matches('\'');
        if matches!(value, ">-" | ">" | "|" | "|-") {
            multiline = Some((key.to_string(), Vec::new()));
            continue;
        }
        if key == "tags" {
            if value.is_empty() {
                collecting_tags = true;
            } else {
                let inner = value.trim_start_matches('[').trim_end_matches(']');
                fm.tags = inner
                    .split(',')
                    .map(|t| t.trim().trim_matches('"').trim_matches('\'').to_string())
                    .filter(|t| !t.is_empty())
                    .collect();
            }
            continue;
        }
        assign(&mut fm, key, value);
    }
    if let Some((key, parts)) = multiline {
        flush(&mut fm, &key, &parts);
    }

    if fm.name.is_empty() {
        return Err(SkillDocError::MissingRequiredField("name"));
    }
    if fm.description.is_empty() {
        return Err(SkillDocError::MissingRequiredField("description"));
    }
    Ok(fm)
}

/// Unknown keys are tolerated and ignored (mirrors zeroclaw `assign`).
fn assign(fm: &mut SkillFrontmatter, key: &str, value: &str) {
    match key {
        "name" => fm.name = value.to_string(),
        "description" => fm.description = value.to_string(),
        "license" => fm.license = Some(value.to_string()),
        "author" => fm.author = Some(value.to_string()),
        "version" => fm.version = Some(value.to_string()),
        "category" => fm.category = Some(value.to_string()),
        "always" => fm.always = value.eq_ignore_ascii_case("true"),
        _ => {}
    }
}

/// Locate the nested `slash_options:` block and return `(start, end)` line
/// indices to skip. Ported shape from zeroclaw `locate_slash_options_block`.
fn locate_slash_options_block(src: &str) -> Option<(usize, usize)> {
    let lines: Vec<&str> = src.lines().collect();
    let start = lines
        .iter()
        .position(|l| l.trim().starts_with("slash_options:"))?;
    let mut end = start + 1;
    while end < lines.len() && (lines[end].starts_with(' ') || lines[end].starts_with('\t')) {
        end += 1;
    }
    Some((start, end))
}

fn write_field(out: &mut String, key: &str, value: &str) {
    if value.contains('\n') {
        write_block_scalar(out, key, value);
    } else {
        let _ = writeln!(out, "{key}: {value}");
    }
}

fn write_block_scalar(out: &mut String, key: &str, value: &str) {
    if value.contains('\n') || value.len() > 80 {
        let _ = writeln!(out, "{key}: >-");
        for line in value.split('\n') {
            let _ = writeln!(out, "  {}", line.trim());
        }
    } else {
        let _ = writeln!(out, "{key}: {value}");
    }
}

fn write_optional(out: &mut String, key: &str, value: Option<&str>) {
    if let Some(v) = value
        && !v.is_empty()
    {
        write_field(out, key, v);
    }
}

/// Serialize tags as inline flow list; omitted when empty (byte-stable).
fn write_tags(out: &mut String, tags: &[String]) {
    if tags.is_empty() {
        return;
    }
    let _ = writeln!(out, "tags: [{}]", tags.join(", "));
}

/// Serialize a bool flag; omitted when false (byte-stable).
fn write_bool(out: &mut String, key: &str, value: bool) {
    if value {
        let _ = writeln!(out, "{key}: true");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(content: &str) -> SkillDocument {
        SkillDocument::parse(content).expect("parse ok")
    }

    #[test]
    fn parses_minimal_and_roundtrips() {
        // Canonical form: a blank line separates frontmatter from body.
        let content = "---\nname: summarize-changes\ndescription: Summarize uncommitted changes.\n---\n\n## Steps\n1. run git diff\n";
        let d = doc(content);
        assert_eq!(d.frontmatter.name, "summarize-changes");
        assert_eq!(d.frontmatter.description, "Summarize uncommitted changes.");
        assert!(d.body.contains("## Steps"));
        assert_eq!(
            d.serialize(),
            content,
            "canonical input round-trips byte-identical"
        );
    }

    #[test]
    fn missing_blank_line_canonicalizes() {
        let d = doc("---\nname: x\ndescription: d\n---\nbody\n");
        assert_eq!(d.body, "body\n");
        let out = d.serialize();
        assert!(
            out.contains("---\n\nbody"),
            "serializer adds canonical blank line"
        );
        assert_eq!(SkillDocument::parse(&out).unwrap(), d, "idempotent");
    }

    #[test]
    fn block_scalar_description_multi_paragraph() {
        // zeroclaw semantics: blank paragraph breaks contribute an empty part
        // joined with single spaces (→ double space between paragraphs).
        let content =
            "---\nname: x\ndescription: >-\n  first paragraph\n  \n  second paragraph\n---\nbody\n";
        let d = doc(content);
        assert_eq!(
            d.frontmatter.description,
            "first paragraph  second paragraph"
        );
    }

    #[test]
    fn parses_tags_inline_and_block() {
        let inline = doc("---\nname: a\ndescription: d\ntags: [x, y]\n---\n");
        assert_eq!(inline.frontmatter.tags, vec!["x", "y"]);
        let block = doc("---\nname: a\ndescription: d\ntags:\n  - x\n  - y\n---\n");
        assert_eq!(block.frontmatter.tags, vec!["x", "y"]);
    }

    #[test]
    fn always_true_and_optional_fields() {
        let d = doc("---\nname: a\ndescription: d\nlicense: MIT\nalways: true\n---\n");
        assert!(d.frontmatter.always);
        assert_eq!(d.frontmatter.license.as_deref(), Some("MIT"));
        let out = d.serialize();
        assert!(out.contains("always: true"));
        assert!(out.contains("license: MIT"));
    }

    #[test]
    fn unknown_keys_tolerated_and_required_fields_enforced() {
        let d = doc("---\nname: a\ndescription: d\nmodel: inherit\n---\n");
        assert_eq!(d.frontmatter.name, "a");
        assert!(SkillDocument::parse("---\ndescription: d\n---\n").is_err());
        assert!(SkillDocument::parse("---\nname: a\n---\n").is_err());
    }

    #[test]
    fn nested_slash_options_do_not_hijack_tags_or_keys() {
        let content = "---\nname: a\ndescription: d\ntags: [keep]\nslash_options:\n  - name: opt\n    description: an option\n    tags: [nested]\n---\nbody\n";
        let d = doc(content);
        assert_eq!(d.frontmatter.tags, vec!["keep"]);
    }

    #[test]
    fn legacy_crlf_normalized_and_missing_frontmatter_error() {
        assert_eq!(
            doc("---\r\nname: a\r\ndescription: d\r\n---\r\nbody\r\n")
                .frontmatter
                .name,
            "a"
        );
        assert!(matches!(
            SkillDocument::parse("no frontmatter\n"),
            Err(SkillDocError::MissingFrontmatter)
        ));
    }
}
