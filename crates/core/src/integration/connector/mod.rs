//! Pull connectors — scheduled fetch + normalize + pipeline (integration.md §5.3).
//!
//! M2 scope: `http-pull` with the generic REST `since_id` cursor template.

pub mod http_pull;

/// One pull execution summary (admin/job-log facing).
#[derive(Debug, Clone, Copy, Default, serde::Serialize)]
pub struct PullSummary {
    /// Items fetched from the remote this run.
    pub fetched: u64,
    /// Newly delivered through the pipeline.
    pub delivered: u64,
    /// Duplicates absorbed by receipts idempotency.
    pub duplicates: u64,
    /// Failed routes (internal retry job takes over; cursor may still advance
    /// past them — recovery is the retry's job, not the cursor's).
    pub failed: u64,
    /// Pages requested.
    pub pages: u64,
}

impl PullSummary {
    /// Whether the run hit the remote at all (false = config/transport error).
    #[must_use]
    pub fn contacted(&self) -> bool {
        self.pages > 0
    }
}
