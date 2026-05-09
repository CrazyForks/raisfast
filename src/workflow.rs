//! 工作流引擎
//!
//! 提供工作流定义管理、实例创建、步骤执行和状态转换。
//!
//! ## 模块结构
//!
//! - [`model`] — 数据结构 + SQL 查询
//! - [`engine`] — 状态机核心（WorkflowService）
//! - [`handler`] — axum 路由处理器
//! - [`validate`] — 步骤定义校验 + 条件表达式评估

pub mod engine;
pub mod handler;
pub mod model;
pub mod validate;

pub use engine::WorkflowService;
pub use model::{StepDef, StepLog, StepType, WorkflowDefinition, WorkflowInstance};
