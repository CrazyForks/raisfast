//! Flow engine scenario test modules.
//!
//! Dedicated corpus for all current + future orchestration scenarios. Add one
//! file per concern (control flow, resilience, dataflow, durable, egress,
//! await/resume, concurrency…) as the engine grows.

pub mod helpers;
pub mod scenario_await;
pub mod scenario_control_flow;
pub mod scenario_resilience;
