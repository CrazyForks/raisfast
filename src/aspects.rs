//! AOP cross-cutting infrastructure
//!
//! Aspect-Oriented Programming framework providing unified cross-cutting abstractions.
//! All data paths (Content Type CRUD, built-in table Services) share the same Aspect dispatch.
//!
//! Four-layer model:
//! - HTTP Layer — request/response interception (Phase 5)
//! - Access Layer — route-level + data-level permissions (Phase 5)
//! - Data Layer — pre/post CRUD interception (core layer, Phase 1)
//! - Event Layer — event publish/consume interception (Phase 3)

pub mod engine;
pub mod excerpt_aspect;
pub mod slug_aspect;

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::content_type::schema::ContentTypeSchema;

// ─── Layer / Operation / When ───

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Layer {
    Http,
    Access,
    Data,
    Event,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Operation {
    Create,
    Read,
    Update,
    Delete,
    Publish,
    Consume,
    Check,
    Filter,
    Request,
    Response,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum When {
    Before,
    After,
}

// ─── JoinPointId ───

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct JoinPointId {
    pub layer: Layer,
    pub operation: Operation,
    pub when: When,
}

// ─── Pointcut ───

#[derive(Debug, Clone)]
pub enum TargetMatcher {
    All,
    Tables(Vec<String>),
}

#[derive(Debug, Clone)]
pub struct Pointcut {
    pub layer: Layer,
    pub operation: Operation,
    pub when: When,
    pub target: TargetMatcher,
}

impl Pointcut {
    fn join_point_id(&self) -> JoinPointId {
        JoinPointId {
            layer: self.layer,
            operation: self.operation,
            when: self.when,
        }
    }
}

// ─── Advice ───

#[derive(Debug)]
pub enum Advice {
    Continue,
    /// Skip remaining Aspects, but the original operation continues (like loop's break).
    /// Use case: skip subsequent Aspect data processing on cache hit.
    Skip,
    Return(Value),
}

pub type AspectResult = Result<Advice, anyhow::Error>;

// ─── Extensions ───

pub struct Extensions {
    map: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
}

impl Clone for Extensions {
    fn clone(&self) -> Self {
        Self {
            map: HashMap::new(),
        }
    }
}

impl Extensions {
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
        }
    }

    pub fn insert<T: Send + Sync + 'static>(&mut self, val: T) {
        self.map.insert(TypeId::of::<T>(), Box::new(val));
    }

    pub fn get<T: 'static>(&self) -> Option<&T> {
        self.map
            .get(&TypeId::of::<T>())
            .and_then(|v| v.downcast_ref::<T>())
    }

    pub fn remove<T: 'static>(&mut self) -> Option<T> {
        self.map
            .remove(&TypeId::of::<T>())
            .and_then(|v| v.downcast::<T>().ok())
            .map(|b| *b)
    }
}

impl Default for Extensions {
    fn default() -> Self {
        Self::new()
    }
}

// ─── BaseContext ───

#[derive(Clone)]
pub struct BaseContext {
    pub user_id: Option<String>,
    pub user_int_id: Option<i64>,
    pub user_role: Option<String>,
    pub tenant_id: String,
    pub now: String,
    pub request_id: String,
    pub extensions: Extensions,
    pub pool: Option<crate::db::pool::Pool>,
}

impl BaseContext {
    pub fn new(user_id: Option<String>, tenant_id: String, now: String) -> Self {
        Self {
            user_id,
            user_int_id: None,
            user_role: None,
            tenant_id,
            now,
            request_id: String::new(),
            extensions: Extensions::new(),
            pool: None,
        }
    }

    pub fn with_pool(mut self, pool: crate::db::pool::Pool) -> Self {
        self.pool = Some(pool);
        self
    }

    pub fn with_user_int_id(mut self, user_int_id: Option<i64>) -> Self {
        self.user_int_id = user_int_id;
        self
    }
}

// ─── Data Layer Contexts ───

pub type Record = serde_json::Map<String, Value>;

/// Result of an aspect data dispatch, with convenience methods for reading fields.
pub struct Dispatched(pub Record);

impl Dispatched {
    pub fn str(&self, key: &str) -> Option<String> {
        self.0
            .get(key)
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }

    pub fn str_or<F: FnOnce() -> String>(&self, key: &str, default: F) -> String {
        self.str(key).unwrap_or_else(default)
    }
}

pub struct DataBeforeCreateContext {
    pub base: BaseContext,
    pub table: String,
    pub record: Record,
    pub schema: Option<Arc<ContentTypeSchema>>,
}

pub struct DataAfterCreateContext {
    pub base: BaseContext,
    pub table: String,
    pub record: Record,
    pub schema: Option<Arc<ContentTypeSchema>>,
}

pub struct DataBeforeReadContext {
    pub base: BaseContext,
    pub table: String,
    pub query: ReadQuery,
    pub schema: Option<Arc<ContentTypeSchema>>,
}

pub struct DataAfterReadContext {
    pub base: BaseContext,
    pub table: String,
    pub records: Vec<Record>,
    pub schema: Option<Arc<ContentTypeSchema>>,
}

pub struct DataBeforeUpdateContext {
    pub base: BaseContext,
    pub table: String,
    pub old_record: Record,
    pub new_record: Record,
    pub schema: Option<Arc<ContentTypeSchema>>,
}

pub struct DataAfterUpdateContext {
    pub base: BaseContext,
    pub table: String,
    pub old_record: Record,
    pub new_record: Record,
    pub schema: Option<Arc<ContentTypeSchema>>,
}

pub struct DataBeforeDeleteContext {
    pub base: BaseContext,
    pub table: String,
    pub record: Record,
    pub soft_delete: bool,
    pub schema: Option<Arc<ContentTypeSchema>>,
}

pub struct DataAfterDeleteContext {
    pub base: BaseContext,
    pub table: String,
    pub record: Record,
    pub schema: Option<Arc<ContentTypeSchema>>,
}

pub struct ReadQuery {
    pub filters: Vec<(String, String)>,
    pub order_by: Option<String>,
    pub page: u64,
    pub page_size: u64,
    pub fields: Option<Vec<String>>,
}

impl Default for ReadQuery {
    fn default() -> Self {
        Self {
            filters: Vec::new(),
            order_by: None,
            page: 1,
            page_size: 25,
            fields: None,
        }
    }
}

// ─── Access Layer Contexts ───

pub struct AccessCheckContext {
    pub base: BaseContext,
    pub route: String,
    pub method: String,
    pub table: Option<String>,
    pub action: String,
}

pub struct AccessFilterContext {
    pub base: BaseContext,
    pub table: String,
    pub conditions: Vec<String>,
    pub params: Vec<String>,
}

// ─── Event Layer Context ───

pub struct EventContext {
    pub base: BaseContext,
    pub event_type: String,
    pub payload: Value,
    pub table: Option<String>,
}

// ─── HTTP Layer Contexts ───

pub struct HttpBeforeContext {
    pub base: BaseContext,
    pub method: String,
    pub path: String,
    pub headers: HashMap<String, String>,
}

pub struct HttpAfterContext {
    pub base: BaseContext,
    pub status_code: u16,
    pub response_body: Option<Value>,
}

// ─── SqlType (re-export) ───

pub use crate::db::sql_type::SqlType;

// ─── ColumnDef ───

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnDef {
    pub name: String,
    pub sql_type: SqlType,
    pub default: Option<String>,
}

// ─── Aspect Trait ───

#[async_trait]
pub trait Aspect: Send + Sync + 'static {
    fn name(&self) -> &str;

    fn priority(&self) -> i32 {
        0
    }

    fn pointcuts(&self) -> Vec<Pointcut>;

    fn columns(&self) -> Vec<ColumnDef> {
        vec![]
    }

    // ─── Data Layer ───

    async fn on_data_before_create(&self, _ctx: &mut DataBeforeCreateContext) -> AspectResult {
        Ok(Advice::Continue)
    }

    async fn on_data_after_create(&self, _ctx: &mut DataAfterCreateContext) -> AspectResult {
        Ok(Advice::Continue)
    }

    async fn on_data_before_read(&self, _ctx: &mut DataBeforeReadContext) -> AspectResult {
        Ok(Advice::Continue)
    }

    async fn on_data_after_read(&self, _ctx: &mut DataAfterReadContext) -> AspectResult {
        Ok(Advice::Continue)
    }

    async fn on_data_before_update(&self, _ctx: &mut DataBeforeUpdateContext) -> AspectResult {
        Ok(Advice::Continue)
    }

    async fn on_data_after_update(&self, _ctx: &mut DataAfterUpdateContext) -> AspectResult {
        Ok(Advice::Continue)
    }

    async fn on_data_before_delete(&self, _ctx: &mut DataBeforeDeleteContext) -> AspectResult {
        Ok(Advice::Continue)
    }

    async fn on_data_after_delete(&self, _ctx: &mut DataAfterDeleteContext) -> AspectResult {
        Ok(Advice::Continue)
    }

    // ─── Access Layer ───

    async fn on_access_check(&self, _ctx: &mut AccessCheckContext) -> AspectResult {
        Ok(Advice::Continue)
    }

    async fn on_access_filter(&self, _ctx: &mut AccessFilterContext) -> AspectResult {
        Ok(Advice::Continue)
    }

    // ─── Event Layer ───

    async fn on_event_before_publish(&self, _ctx: &mut EventContext) -> AspectResult {
        Ok(Advice::Continue)
    }

    async fn on_event_after_publish(&self, _ctx: &mut EventContext) -> AspectResult {
        Ok(Advice::Continue)
    }

    async fn on_event_before_consume(&self, _ctx: &mut EventContext) -> AspectResult {
        Ok(Advice::Continue)
    }

    async fn on_event_after_consume(&self, _ctx: &mut EventContext) -> AspectResult {
        Ok(Advice::Continue)
    }

    // ─── HTTP Layer ───

    async fn on_http_before(&self, _ctx: &mut HttpBeforeContext) -> AspectResult {
        Ok(Advice::Continue)
    }

    async fn on_http_after(&self, _ctx: &mut HttpAfterContext) -> AspectResult {
        Ok(Advice::Continue)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions_insert_get_remove() {
        let mut ext = Extensions::new();
        assert!(ext.get::<String>().is_none());

        ext.insert::<String>("hello".into());
        assert_eq!(ext.get::<String>().unwrap(), "hello");

        let removed = ext.remove::<String>();
        assert_eq!(removed.unwrap(), "hello");
        assert!(ext.get::<String>().is_none());
    }

    #[test]
    fn extensions_different_types() {
        let mut ext = Extensions::new();
        ext.insert::<i32>(42);
        ext.insert::<String>("world".into());
        assert_eq!(*ext.get::<i32>().unwrap(), 42);
        assert_eq!(ext.get::<String>().unwrap(), "world");
    }

    #[test]
    fn extensions_overwrite() {
        let mut ext = Extensions::new();
        ext.insert::<i32>(1);
        ext.insert::<i32>(2);
        assert_eq!(*ext.get::<i32>().unwrap(), 2);
    }

    #[test]
    fn extensions_remove_missing() {
        let mut ext = Extensions::new();
        assert!(ext.remove::<i32>().is_none());
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct OwnableData {
        created_by: String,
    }

    #[test]
    fn extensions_custom_type() {
        let mut ext = Extensions::new();
        ext.insert(OwnableData {
            created_by: "user-1".into(),
        });
        let data = ext.get::<OwnableData>().unwrap();
        assert_eq!(data.created_by, "user-1");
    }

    #[test]
    fn base_context_construction() {
        let ctx = BaseContext::new(
            Some("user-1".into()),
            "tenant-1".into(),
            "2026-01-01T00:00:00Z".into(),
        );
        assert_eq!(ctx.user_id.as_deref(), Some("user-1"));
        assert_eq!(ctx.tenant_id, "tenant-1");
        assert_eq!(ctx.now, "2026-01-01T00:00:00Z");
        assert!(ctx.user_role.is_none());
        assert!(ctx.request_id.is_empty());
    }

    #[test]
    fn base_context_with_extensions() {
        let mut ctx = BaseContext::new(None, "default".into(), "now".into());
        ctx.extensions.insert::<String>("test-value".into());
        assert_eq!(ctx.extensions.get::<String>().unwrap(), "test-value");
    }

    #[test]
    fn join_point_id_equality() {
        let a = JoinPointId {
            layer: Layer::Data,
            operation: Operation::Create,
            when: When::Before,
        };
        let b = JoinPointId {
            layer: Layer::Data,
            operation: Operation::Create,
            when: When::Before,
        };
        let c = JoinPointId {
            layer: Layer::Data,
            operation: Operation::Create,
            when: When::After,
        };
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn join_point_id_hash() {
        use std::collections::HashSet;
        let a = JoinPointId {
            layer: Layer::Data,
            operation: Operation::Create,
            when: When::Before,
        };
        let b = JoinPointId {
            layer: Layer::Data,
            operation: Operation::Create,
            when: When::Before,
        };
        let mut set = HashSet::new();
        set.insert(a.clone());
        assert!(set.contains(&b));
    }

    #[test]
    fn pointcut_join_point_id() {
        let pc = Pointcut {
            layer: Layer::Data,
            operation: Operation::Update,
            when: When::After,
            target: TargetMatcher::All,
        };
        let jpid = pc.join_point_id();
        assert_eq!(jpid.layer, Layer::Data);
        assert_eq!(jpid.operation, Operation::Update);
        assert_eq!(jpid.when, When::After);
    }

    #[test]
    fn layer_operation_when_variants() {
        assert_ne!(Layer::Http, Layer::Data);
        assert_ne!(Operation::Create, Operation::Read);
        assert_ne!(When::Before, When::After);
    }

    #[test]
    fn read_query_default() {
        let q = ReadQuery::default();
        assert!(q.filters.is_empty());
        assert!(q.order_by.is_none());
        assert_eq!(q.page, 1);
        assert_eq!(q.page_size, 25);
        assert!(q.fields.is_none());
    }

    #[test]
    fn column_def_construction() {
        let col = ColumnDef {
            name: "created_by".into(),
            sql_type: SqlType::Text,
            default: None,
        };
        assert_eq!(col.name, "created_by");
        assert_eq!(col.sql_type, SqlType::Text);
        assert_eq!(col.sql_type.as_str(), "TEXT");
        assert!(col.default.is_none());
    }

    #[test]
    fn sql_type_as_str() {
        assert_eq!(SqlType::Text.as_str(), "TEXT");
        assert_eq!(SqlType::Integer.as_str(), "INTEGER");
        assert_eq!(SqlType::Boolean.as_str(), "BOOLEAN");
    }

    #[test]
    fn advice_variants() {
        let a = Advice::Continue;
        let b = Advice::Skip;
        let c = Advice::Return(serde_json::json!({"key": "val"}));
        assert!(matches!(a, Advice::Continue));
        assert!(matches!(b, Advice::Skip));
        assert!(matches!(c, Advice::Return(_)));
    }

    #[test]
    fn record_operations() {
        let mut r = Record::new();
        r.insert("id".into(), serde_json::json!("abc"));
        r.insert("count".into(), serde_json::json!(42));
        assert_eq!(r.get("id").unwrap(), &serde_json::json!("abc"));
        assert_eq!(r.get("count").unwrap(), &serde_json::json!(42));
        assert!(r.get("missing").is_none());
        r.remove("count");
        assert!(r.get("count").is_none());
    }
}
