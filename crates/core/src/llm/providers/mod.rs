//! Native-protocol providers for internal consumption — one provider, one
//! file (media-nodes §5 discipline applied to the llm domain; the set will
//! grow to dozens, hence the directory).
//!
//! Contract (see `anthropic.rs` header for the canonical reference matrix):
//! - constructor surface `(base_url, api_key, param_override, header_override)`;
//! - implement ONLY the modalities the upstream natively speaks — everything
//!   else keeps the trait defaults (`ProviderError::Config`) so the kernel
//!   skips the channel (failover, no failure report);
//! - error envelopes are compacted to one-line bodies for readable logs.

pub mod anthropic;
pub mod elevenlabs;
pub mod kling;
pub mod minimax;
pub mod replicate;
pub mod seedance;

pub use anthropic::AnthropicProvider;
pub use elevenlabs::ElevenLabsProvider;
pub use kling::KlingProvider;
pub use minimax::MiniMaxProvider;
pub use replicate::ReplicateProvider;
pub use seedance::SeedanceProvider;

/// Map a `WxH` size string to the nearest aspect-ratio enum label shared by
/// video providers (Kling/Replicate input vocab): `16:9` / `1:1` / `9:16`.
/// Absent when unparseable — upstream default applies.
#[must_use]
pub(crate) fn aspect_ratio_from_size(size: Option<&str>) -> Option<String> {
    let (w, h) = size?.split_once('x')?;
    let w: f64 = w.trim().parse().ok()?;
    let h: f64 = h.trim().parse().ok()?;
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    let ratio = w / h;
    let candidates = [(16.0 / 9.0, "16:9"), (1.0, "1:1"), (9.0 / 16.0, "9:16")];
    let (_, label) = candidates
        .iter()
        .min_by(|a, b| (a.0 - ratio).abs().total_cmp(&(b.0 - ratio).abs()))?;
    Some((*label).to_string())
}
