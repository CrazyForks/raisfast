//! Workflow engine
//!
//! Provides workflow definition management, instance creation, step execution, and state transitions.
//!
//! ## Module structure
//!
//! - [`model`] — Data structures + SQL queries
//! - [`engine`] — State machine core (WorkflowService)
//! - [`handler`] — Axum route handlers
//! - [`validate`] — Step definition validation + condition expression evaluation

pub mod engine;
pub mod handler;
pub mod model;
pub mod validate;

pub use engine::WorkflowService;
pub use model::{StepDef, StepLog, StepType, WorkflowDefinition, WorkflowInstance};

#[cfg(feature = "export-types")]
export_types!(
    handler::CreateWorkflowRequest,
    handler::StartWorkflowRequest,
    handler::ExecuteStepRequest,
    handler::InstanceQuery,
    model::WorkflowInstanceStatus,
    model::WorkflowStepStatus,
    StepType,
    StepDef,
    WorkflowDefinition,
    WorkflowInstance,
    StepLog,
);
