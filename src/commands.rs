//! Cross-layer shared write operation Command objects
//!
//! Commands encapsulate all parameters for Repository write operations, replacing multi-parameter function signatures.
//! All layers (handlers, services, repositories, models) can reference them.

pub mod cart;
pub mod category;
pub mod comment;
pub mod coupon;
pub mod media;
pub mod order;
pub mod page;
pub mod payment;
pub mod post;
pub mod product;
pub mod product_category;
pub mod product_comment;
pub mod product_variant;
pub mod rbac;
pub mod reusable_block;
pub mod shipping_template;
pub mod user;
pub mod user_address;
pub mod wallet;

pub use cart::*;
pub use category::*;
pub use comment::*;
pub use coupon::*;
pub use media::*;
pub use order::*;
pub use page::*;
pub use payment::*;
pub use post::*;
pub use product::*;
pub use product_category::*;
pub use product_comment::*;
pub use product_variant::*;
pub use rbac::*;
pub use reusable_block::*;
pub use shipping_template::*;
pub use user::*;
pub use user_address::*;
pub use wallet::*;
