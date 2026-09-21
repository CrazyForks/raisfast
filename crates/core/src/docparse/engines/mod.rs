//! 解析引擎实现层——每个文件一个引擎，统一实现根模块的
//! `ParseEngine` trait [抄WK:docparser/engines.go 多引擎注册]。

pub mod builtin;
pub mod docreader;
pub mod docreader_proto;
pub mod mineru;
pub mod mineru_cloud;
pub mod paddleocr_vl;
pub mod paddleocr_vl_cloud;

// 引擎文件经 `use super::X` 引用根类型——此处再导出保持解析不变。
pub use super::validate_external_url;
pub use crate::docparse::{ParseEngine, ParseOpts, ParseOutcome, ParsedImage};
