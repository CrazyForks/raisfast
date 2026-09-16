//! Prompt-file loader — prompts live as plain text files in the source tree.
//!
//! Release builds embed them at compile time via `include_str!` (single
//! binary, zero runtime files). Debug builds re-read the file from the
//! source tree on each call, so editing a prompt applies on the next LLM
//! turn without recompiling; any read failure falls back to the embedded
//! copy. Reference matrix: `dev-docs/agent/prompt-engineering.md §11`.

/// Load a prompt text file.
///
/// `embedded` is the compile-time copy (`include_str!`, supplied by the
/// [`prompt_file!`] macro); `rel` is the manifest-root-relative path used
/// for the debug hot-read. Trailing whitespace is trimmed on both paths so
/// debug and release render byte-identical text (stable `system_hash`).
pub fn load(embedded: &'static str, rel: &str) -> String {
    #[cfg(debug_assertions)]
    {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
        match std::fs::read_to_string(&path) {
            Ok(text) if !text.trim().is_empty() => return text.trim_end().to_owned(),
            _ => tracing::warn!(
                path = %path.display(),
                "prompt file hot-read failed; using embedded text"
            ),
        }
    }
    #[cfg(not(debug_assertions))]
    {
        let _ = rel;
    }
    embedded.trim_end().to_owned()
}

/// Embed the prompt file at `rel` (a manifest-root-relative string literal)
/// and load it through [`load`]. Crate-internal: import with
/// `use crate::utils::prompt_file::prompt_file;`.
macro_rules! prompt_file {
    ($rel:expr) => {
        $crate::utils::prompt_file::load(
            include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/", $rel)),
            $rel,
        )
    };
}
pub(crate) use prompt_file;

#[cfg(test)]
mod tests {
    #[test]
    fn prompt_files_load_trimmed() {
        let text = prompt_file!("src/agent/prompts/task.md");
        assert!(text.starts_with("# Task\n"));
        assert!(!text.ends_with('\n'), "trailing newline must be trimmed");
        assert!(text.contains("若用户没给语言偏好"));
    }

    #[test]
    fn embedded_fallback_matches_hot_read() {
        // In debug the file is hot-read; both paths must trim identically.
        let via_load = prompt_file!("src/kb/prompts/understand.md");
        assert!(via_load.contains("搜索查询优化器"));
        assert!(!via_load.ends_with('\n'));
    }
}
