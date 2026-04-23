//! 跨层共享的写操作 Command 对象
//!
//! Command 封装 Repository 写操作的所有参数，替代多参数函数签名。
//! 所有层（handlers、services、repositories、models）均可引用。

pub mod category;
pub mod comment;
pub mod media;
pub mod page;
pub mod post;
pub mod user;

pub use category::*;
pub use comment::*;
pub use media::*;
pub use page::*;
pub use post::*;
pub use user::*;
