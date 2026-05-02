# AOP 框架设计文档 — v2.0 实现版

> 版本：v2.0 | 最后更新：2026-05-01
> 状态：Phase 1 + Phase 3 已实现，906 tests 全部通过

## 1. 为什么需要 AOP

当前系统的横切面逻辑散落在四层中，没有统一抽象：

```
一个 POST /cms/articles 实际经过的逻辑：

Middleware 层
  ├─ CORS 检查
  ├─ Rate Limit
  ├─ JWT 解析

Handler 层
  ├─ access control 检查         ← handler 内 if 判断
  ├─ 参数校验                   ← validation.rs
  ├─ 缓存检查（read 时）        ← handler 内 if cache
  ├─ 缓存清除（write 后）       ← handler 内 if cache
  ├─ 字段过滤（response）       ← filter_fields()
  │
Repository 层
  ├─ auto_fill 注入             ← save_ctx
  ├─ timestamps 注入            ← if ct.timestamps
  ├─ 软删除处理                 ← if ct.soft_delete
  ├─ 版本快照                   ← if ct.versioning
  ├─ INSERT/UPDATE/DELETE SQL
  │
EventBus 层
  ├─ 审计日志                   ← subscriber
  ├─ Webhook 通知               ← subscriber
  └─ 搜索索引                   ← 未实现
```

**问题：** 内置表（posts/pages/comments）的横切面逻辑全靠手写，无法复用 Content Type 的任何机制。每新增一个横切面需求，要在多个地方重复实现。

**目标：** 每个 Aspect（横切面）定义一次，所有数据路径自动生效。

## 2. 概念模型

```
┌──────────────────────────────────────────────────────────┐
│                        Aspect                            │
│  "我叫什么"            (name)                             │
│  "我关心什么事件"     (pointcut)                          │
│  "我排在第几位"         (priority)                        │
│  "我需要什么系统列"     (columns)                          │
│  "我做什么"                      (advise)                 │
├──────────────────────────────────────────────────────────┤
│  Protocol    Aspect 的命名别名，1:N 组合多个 Aspect       │
│  Layer       Aspect 关心的系统层级                         │
│  Pointcut    Aspect 关心的拦截点（精确匹配或模式匹配）      │
│  Advice      Aspect 在拦截点上的具体行为                    │
│  Context     拦截点的上下文数据（可读/可写）                 │
│  Engine      调度器，管理 Aspect 注册和执行                  │
└──────────────────────────────────────────────────────────┘
```

## 3. 三层架构

```
┌─ aspects/ ─────────────────────────────────────────────┐
│  纯框架层，不知道 Protocol 存在                          │
│  Aspect trait + AspectEngine + Context 类型 + Advice    │
└─────────────────────────────────────────────────────────┘
          ▲
          │ register_from_arc()
┌─ protocols/ ───────────────────────────────────────────┐
│  业务 Protocol 实现（1:N 组合 Aspect）                   │
│  Protocol trait + ProtocolRegistry                     │
│  ownable.rs / timestampable.rs / ...                   │
└─────────────────────────────────────────────────────────┘
          ▲
          │ AspectDispatch helper
┌─ services/aspect_dispatch.rs ──────────────────────────┐
│  内置表 Service 层的轻量 dispatch helper                 │
│  减少 Service 层的重复代码                               │
└─────────────────────────────────────────────────────────┘
```

## 4. Layer（系统层级）

```
┌─ HTTP Layer ──────────────────────────────────────────┐
│  HTTP 请求/响应拦截                                     │
│  适用：CORS、Rate Limit、请求日志、请求追踪             │
│  状态：Phase 5，未实现                                  │
└───────────────────────────────────────────────────────┘

┌─ Access Layer ────────────────────────────────────────┐
│  路由级 + 数据级权限检查                                │
│  适用：角色校验、RBAC、API Rule 数据过滤、字段级 ACL     │
│  状态：Phase 5，未实现                                  │
└───────────────────────────────────────────────────────┘

┌─ Data Layer ──────────────────────────────────────────┐
│  数据 CRUD 操作前后拦截                                 │
│  适用：字段注入、校验、版本快照、缓存、搜索索引          │
│  状态：Phase 1 + Phase 3，已实现 ✅                     │
└───────────────────────────────────────────────────────┘

┌─ Event Layer ─────────────────────────────────────────┐
│  事件发布/消费拦截（异步，事务外）                       │
│  适用：审计日志、Webhook、通知、搜索索引更新              │
│  状态：未实现                                           │
└───────────────────────────────────────────────────────┘
```

### 4.1 同步 vs 异步边界

| 层 | 执行方式 | 事务 | 失败影响 |
|---|---|---|---|
| HTTP | 同步 | 无 | 阻断请求 |
| Access | 同步 | 无 | 阻断请求 |
| Data Before | 同步 | **在事务内** | 回滚整个操作 |
| Data After | 同步 | **在事务内** | 回滚整个操作 |
| Event | 异步 | 事务外 | 仅记日志，不影响主操作 |

## 5. 核心数据模型

### 5.1 Layer / Operation / When

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Layer { Http, Access, Data, Event }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Operation {
    Create, Read, Update, Delete,
    Publish, Consume, Check, Filter, Request, Response,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum When { Before, After }
```

### 5.2 JoinPointId

```rust
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct JoinPointId {
    pub layer: Layer,
    pub operation: Operation,
    pub when: When,
}
```

### 5.3 TargetMatcher（已简化）

v2.0 只保留两种匹配方式：

```rust
#[derive(Debug, Clone)]
pub enum TargetMatcher {
    /// 匹配所有目标
    All,
    /// 精确匹配表名列表
    Tables(Vec<String>),
}
```

v1.0 中的 `TablePattern`、`Routes`、`Events`、`Custom`、`TargetInfo` 已删除，因为 Phase 1 只需要 `All` 和 `Tables`。

### 5.4 Pointcut

```rust
#[derive(Debug, Clone)]
pub struct Pointcut {
    pub layer: Layer,
    pub operation: Operation,
    pub when: When,
    pub target: TargetMatcher,
}

impl Pointcut {
    fn join_point_id(&self) -> JoinPointId { ... }
}
```

### 5.5 Advice（三态语义）

```rust
#[derive(Debug)]
pub enum Advice {
    /// 继续执行下一个 Aspect
    Continue,
    /// 跳过剩余 Aspect（break），原始操作继续执行
    Skip,
    /// 短路返回（仅 before hook 生效）
    Return(serde_json::Value),
}

pub type AspectResult = Result<Advice, anyhow::Error>;
```

**dispatch 行为：**

| 场景 | before dispatch | after dispatch |
|---|---|---|
| `Ok(Continue)` | 继续下一个 Aspect | 继续下一个 Aspect |
| `Ok(Skip)` | **break**，跳过剩余 Aspect | 不适用 |
| `Ok(Return(val))` | 短路返回 `Some(val)` | 不适用 |
| `Ok(_)` (after hook) | — | 全部 continue |
| `Err(e)` | 中断，返回 Err | 中断，返回 Err |

### 5.6 SqlType（跨 DB 抽象）

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SqlType {
    Text, Integer, BigInt, Real, Boolean, Blob,
}

impl SqlType {
    /// 按 cfg feature 返回对应数据库的 SQL 类型字符串
    pub fn as_str(self) -> &'static str {
        // SQLite:  TEXT / INTEGER / REAL / BOOLEAN / BLOB
        // PostgreSQL: TEXT / INTEGER / BIGINT / DOUBLE PRECISION / BOOLEAN / BYTEA
        // MySQL: VARCHAR(255) / INT / BIGINT / DOUBLE / TINYINT(1) / BLOB
    }
}
```

### 5.7 ColumnDef

```rust
#[derive(Debug, Clone)]
pub struct ColumnDef {
    pub name: String,
    pub sql_type: SqlType,  // v2.0: 从 String 改为 SqlType 枚举
    pub default: Option<String>,
}
```

## 6. Aspect trait（胖 trait 模式）

```rust
#[async_trait]
pub trait Aspect: Send + Sync + 'static {
    fn name(&self) -> &str;
    fn priority(&self) -> i32 { 0 }
    fn pointcuts(&self) -> Vec<Pointcut>;
    fn columns(&self) -> Vec<ColumnDef> { vec![] }

    // ─── Data Layer（已实现）───
    async fn on_data_before_create(&self, _ctx: &mut DataBeforeCreateContext) -> AspectResult { Ok(Advice::Continue) }
    async fn on_data_after_create(&self, _ctx: &mut DataAfterCreateContext) -> AspectResult { Ok(Advice::Continue) }
    async fn on_data_before_read(&self, _ctx: &mut DataBeforeReadContext) -> AspectResult { Ok(Advice::Continue) }
    async fn on_data_after_read(&self, _ctx: &mut DataAfterReadContext) -> AspectResult { Ok(Advice::Continue) }
    async fn on_data_before_update(&self, _ctx: &mut DataBeforeUpdateContext) -> AspectResult { Ok(Advice::Continue) }
    async fn on_data_after_update(&self, _ctx: &mut DataAfterUpdateContext) -> AspectResult { Ok(Advice::Continue) }
    async fn on_data_before_delete(&self, _ctx: &mut DataBeforeDeleteContext) -> AspectResult { Ok(Advice::Continue) }
    async fn on_data_after_delete(&self, _ctx: &mut DataAfterDeleteContext) -> AspectResult { Ok(Advice::Continue) }

    // ─── Access Layer（Phase 5）───
    async fn on_access_check(&self, _ctx: &mut AccessCheckContext) -> AspectResult { Ok(Advice::Continue) }
    async fn on_access_filter(&self, _ctx: &mut AccessFilterContext) -> AspectResult { Ok(Advice::Continue) }

    // ─── Event Layer（Phase 4）───
    async fn on_event_before_publish(&self, _ctx: &mut EventContext) -> AspectResult { Ok(Advice::Continue) }
    async fn on_event_after_publish(&self, _ctx: &mut EventContext) -> AspectResult { Ok(Advice::Continue) }
    async fn on_event_before_consume(&self, _ctx: &mut EventContext) -> AspectResult { Ok(Advice::Continue) }
    async fn on_event_after_consume(&self, _ctx: &mut EventContext) -> AspectResult { Ok(Advice::Continue) }

    // ─── HTTP Layer（Phase 5）───
    async fn on_http_before(&self, _ctx: &mut HttpBeforeContext) -> AspectResult { Ok(Advice::Continue) }
    async fn on_http_after(&self, _ctx: &mut HttpAfterContext) -> AspectResult { Ok(Advice::Continue) }
}
```

### 6.1 为什么用 "胖 trait"

1. **统一注册：** `Vec<Arc<dyn Aspect>>` 一个列表管理所有 Aspect
2. **跨层 Aspect：** 一个 Aspect 可以同时拦截 HTTP + Data 层（如安全审计）
3. **简洁：** 不需要多 trait object 转换
4. **性能可接受：** Engine 在注册时通过 `pointcuts()` 预过滤，运行时只调用匹配的 hook

## 7. Context 类型体系

### 7.1 BaseContext

```rust
pub struct BaseContext {
    pub user_id: Option<String>,
    pub user_role: Option<String>,
    pub tenant_id: String,
    pub now: String,           // ISO 8601
    pub request_id: String,
    pub extensions: Extensions,
    pub pool: Option<crate::db::pool::Pool>,  // v2.0: 直接持有 Pool
}
```

`pool` 字段让 Aspect 可以在 hook 内执行数据库操作（如版本快照）。

### 7.2 Extensions（Aspect 间通信）

```rust
pub struct Extensions {
    map: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
}

impl Extensions {
    pub fn insert<T: Send + Sync + 'static>(&mut self, val: T);
    pub fn get<T: 'static>(&self) -> Option<&T>;
    pub fn remove<T: 'static>(&mut self) -> Option<T>;
}
```

### 7.3 Data Layer Contexts

```rust
pub type Record = serde_json::Map<String, Value>;

pub struct DataBeforeCreateContext {
    pub base: BaseContext,
    pub table: String,
    pub record: Record,              // 可修改
    pub schema: Option<Arc<ContentTypeSchema>>,
}

pub struct DataAfterCreateContext {
    pub base: BaseContext,
    pub table: String,
    pub record: Record,              // 只读
    pub schema: Option<Arc<ContentTypeSchema>>,
}

pub struct DataBeforeReadContext {
    pub base: BaseContext,
    pub table: String,
    pub query: ReadQuery,            // 可修改
    pub schema: Option<Arc<ContentTypeSchema>>,
}

pub struct DataAfterReadContext {
    pub base: BaseContext,
    pub table: String,
    pub records: Vec<Record>,        // 可修改
    pub schema: Option<Arc<ContentTypeSchema>>,
}

pub struct DataBeforeUpdateContext {
    pub base: BaseContext,
    pub table: String,
    pub old_record: Record,          // 只读
    pub new_record: Record,          // 可修改
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
    pub record: Record,              // 只读
    pub soft_delete: bool,           // Aspect 可设为 true
    pub schema: Option<Arc<ContentTypeSchema>>,
}

pub struct DataAfterDeleteContext {
    pub base: BaseContext,
    pub table: String,
    pub record: Record,
    pub schema: Option<Arc<ContentTypeSchema>>,
}
```

### 7.4 ReadQuery

```rust
pub struct ReadQuery {
    pub filters: Vec<(String, String)>,
    pub order_by: Option<String>,
    pub page: u64,
    pub page_size: u64,
    pub fields: Option<Vec<String>>,
}
```

### 7.5 Access / Event / HTTP Layer Contexts

```rust
pub struct AccessCheckContext {
    pub base: BaseContext,
    pub route: String, pub method: String, pub table: Option<String>, pub action: String,
}

pub struct AccessFilterContext {
    pub base: BaseContext,
    pub table: String, pub conditions: Vec<String>, pub params: Vec<String>,
}

pub struct EventContext {
    pub base: BaseContext,
    pub event_type: String, pub payload: Value, pub table: Option<String>,
}

pub struct HttpBeforeContext {
    pub base: BaseContext,
    pub method: String, pub path: String, pub headers: HashMap<String, String>,
}

pub struct HttpAfterContext {
    pub base: BaseContext,
    pub status_code: u16, pub response_body: Option<Value>,
}
```

## 8. AspectEngine

### 8.1 数据结构

```rust
pub struct AspectEngine {
    /// JoinPointId → 排序后的 Aspect 列表（注册时预计算，运行时 O(1) 查找）
    dispatch_table: DashMap<JoinPointId, Vec<Arc<dyn Aspect>>>,
    /// 所有 Aspect 的注册信息
    registry: RwLock<Vec<AspectEntry>>,
}

pub struct AspectEntry {
    pub aspect: Arc<dyn Aspect>,
    pub pointcuts: Vec<Pointcut>,
    pub enabled: bool,
}
```

### 8.2 注册

```rust
impl AspectEngine {
    pub fn new() -> Self;

    /// 从具体类型注册
    pub fn register(&self, aspect: impl Aspect);

    /// 从 Arc 注册（Protocol 层使用）
    pub fn register_from_arc(&self, arc: Arc<dyn Aspect>);
}
```

注册流程：
1. 读取 `pointcuts()` 确定关心的 JoinPointId
2. 按 `priority()` 插入到 `dispatch_table` 对应列表
3. 保存到 `registry`

### 8.3 运行时开关

```rust
impl AspectEngine {
    /// 启用指定 Aspect（按名称查找）
    pub fn enable(&self, name: &str) -> bool;
    /// 禁用指定 Aspect（按名称查找）
    pub fn disable(&self, name: &str) -> bool;
}
```

### 8.4 查询

```rust
impl AspectEngine {
    /// 返回所有 enabled 的 Aspect
    pub fn aspects(&self) -> Vec<Arc<dyn Aspect>>;

    /// 返回指定表相关的系统列（去重）
    pub fn columns_for(&self, table: &str) -> Vec<ColumnDef>;
}
```

### 8.5 调度方法

```rust
impl AspectEngine {
    // Data Layer — before 返回 Option<Value>（短路值），after 返回 ()
    pub async fn dispatch_data_before_create(&self, table: &str, ctx: &mut DataBeforeCreateContext) -> Result<Option<Value>>;
    pub async fn dispatch_data_after_create(&self, table: &str, ctx: &mut DataAfterCreateContext) -> Result<()>;
    pub async fn dispatch_data_before_read(&self, table: &str, ctx: &mut DataBeforeReadContext) -> Result<Option<Value>>;
    pub async fn dispatch_data_after_read(&self, table: &str, ctx: &mut DataAfterReadContext) -> Result<()>;
    pub async fn dispatch_data_before_update(&self, table: &str, ctx: &mut DataBeforeUpdateContext) -> Result<Option<Value>>;
    pub async fn dispatch_data_after_update(&self, table: &str, ctx: &mut DataAfterUpdateContext) -> Result<()>;
    pub async fn dispatch_data_before_delete(&self, table: &str, ctx: &mut DataBeforeDeleteContext) -> Result<Option<Value>>;
    pub async fn dispatch_data_after_delete(&self, table: &str, ctx: &mut DataAfterDeleteContext) -> Result<()>;
}
```

### 8.6 get_aspects 内部实现

```rust
fn get_aspects(&self, jp_id: &JoinPointId, table: &str) -> Vec<Arc<dyn Aspect>> {
    // 1. 收集 enabled 名称为 HashSet<String>（拥有所有权，释放 RwLockReadGuard）
    let enabled_names: HashSet<String> = self.registry.read().unwrap()
        .iter().filter(|e| e.enabled).map(|e| e.aspect.name().to_string()).collect();

    // 2. 从 dispatch_table 查找匹配的 Aspect
    let Some(aspects) = self.dispatch_table.get(jp_id) else { return Vec::new() };
    aspects.iter()
        .filter(|a| enabled_names.contains(a.name()) && matches_table_any(&a.pointcuts(), table))
        .cloned().collect()
}
```

### 8.7 优先级约定

```
优先级范围        用途                           示例
─────────────────────────────────────────────────────────
 -9999 ~ -1000   核心基础设施                   认证、租户隔离
  -999 ~ -500    数据注入                       ownable (-500), timestampable (-400)
  -499 ~    0    默认
     1 ~  499    业务逻辑                       validation, slug 生成
   500 ~  999    副作用（事务内）                版本快照、搜索索引
  1000 ~  9999   事后处理（事务内）              缓存清除
 10000+          非关键（可降级）                统计、监控
```

## 9. Protocol 层

### 9.1 概念

Protocol = 一组 Aspect 的命名别名（1:N 关系）。是 AOP 之上的薄声明层。

```
ContentTypeSchema.implements = ["ownable", "timestampable"]
                         │
                         ▼
ProtocolRegistry.get("ownable")
  ├─ name: "ownable"
  ├─ description: "创建和更新时自动注入操作者 ID"
  ├─ aspects: [OwnableAspect]
  ├─ columns: [created_by, updated_by]
  └─ built_in: true
```

### 9.2 Protocol trait

```rust
pub trait Protocol: Send + Sync + 'static {
    fn name(&self) -> &str;
    fn description(&self) -> &str { "" }
    fn aspects(&self) -> Vec<Arc<dyn Aspect>>;
    fn columns(&self) -> Vec<ColumnDef> {
        // 默认从 aspects 聚合
        self.aspects().iter().flat_map(|a| a.columns()).collect()
    }
    fn built_in(&self) -> bool { false }
}
```

### 9.3 ProtocolRegistry

```rust
pub struct ProtocolRegistry {
    protocols: HashMap<String, Arc<dyn Protocol>>,
}

impl ProtocolRegistry {
    pub fn new() -> Self;
    pub fn register(&mut self, protocol: impl Protocol);
    pub fn get(&self, name: &str) -> Option<&Arc<dyn Protocol>>;
    pub fn names(&self) -> Vec<&str>;

    /// 按名称查询列（自动去重）
    pub fn columns_for(&self, names: &[String]) -> Vec<ColumnDef>;

    /// 按名称查询 Aspect（自动去重）
    pub fn aspects_for(&self, names: &[String]) -> Vec<Arc<dyn Aspect>>;

    /// 将所有 Protocol 的 Aspect 注册到 AspectEngine（去重）
    pub fn register_aspects_into(&self, engine: &AspectEngine);
}
```

## 10. 内置 Protocol 实现

### 10.1 ownable

**文件：** `src/protocols/ownable.rs`

```rust
pub struct OwnableAspect;

impl Aspect for OwnableAspect {
    fn name(&self) -> &str { "ownable" }
    fn priority(&self) -> i32 { -500 }

    fn pointcuts(&self) -> Vec<Pointcut> {
        vec![
            // Create: 注入 created_by + updated_by
            Pointcut { layer: Layer::Data, operation: Operation::Create, when: When::Before, target: TargetMatcher::All },
            // Update: 注入 updated_by
            Pointcut { layer: Layer::Data, operation: Operation::Update, when: When::Before, target: TargetMatcher::All },
        ]
    }

    fn columns(&self) -> Vec<ColumnDef> {
        vec![
            ColumnDef { name: "created_by".into(), sql_type: SqlType::Text, default: None },
            ColumnDef { name: "updated_by".into(), sql_type: SqlType::Text, default: None },
        ]
    }

    async fn on_data_before_create(&self, ctx: &mut DataBeforeCreateContext) -> AspectResult {
        if let Some(user_id) = &ctx.base.user_id {
            ctx.record.insert("created_by".into(), json!(user_id));
            ctx.record.insert("updated_by".into(), json!(user_id));
        }
        Ok(Advice::Continue)
    }

    async fn on_data_before_update(&self, ctx: &mut DataBeforeUpdateContext) -> AspectResult {
        if let Some(user_id) = &ctx.base.user_id {
            ctx.new_record.insert("updated_by".into(), json!(user_id));
        }
        Ok(Advice::Continue)
    }
}

pub struct OwnableProtocol;

impl Protocol for OwnableProtocol {
    fn name(&self) -> &str { "ownable" }
    fn description(&self) -> &str { "创建和更新时自动注入操作者 ID" }
    fn aspects(&self) -> Vec<Arc<dyn Aspect>> { vec![Arc::new(OwnableAspect)] }
    fn built_in(&self) -> bool { true }
}
```

### 10.2 timestampable

**文件：** `src/protocols/timestampable.rs`

```rust
pub struct TimestampableAspect;

impl Aspect for TimestampableAspect {
    fn name(&self) -> &str { "timestampable" }
    fn priority(&self) -> i32 { -400 }

    fn pointcuts(&self) -> Vec<Pointcut> {
        vec![
            Pointcut { layer: Layer::Data, operation: Operation::Create, when: When::Before, target: TargetMatcher::All },
            Pointcut { layer: Layer::Data, operation: Operation::Update, when: When::Before, target: TargetMatcher::All },
        ]
    }

    fn columns(&self) -> Vec<ColumnDef> {
        vec![
            ColumnDef { name: "created_at".into(), sql_type: SqlType::Text, default: None },
            ColumnDef { name: "updated_at".into(), sql_type: SqlType::Text, default: None },
        ]
    }

    async fn on_data_before_create(&self, ctx: &mut DataBeforeCreateContext) -> AspectResult {
        ctx.record.insert("created_at".into(), json!(ctx.base.now));
        ctx.record.insert("updated_at".into(), json!(ctx.base.now));
        Ok(Advice::Continue)
    }

    async fn on_data_before_update(&self, ctx: &mut DataBeforeUpdateContext) -> AspectResult {
        ctx.new_record.insert("updated_at".into(), json!(ctx.base.now));
        Ok(Advice::Continue)
    }
}

pub struct TimestampableProtocol;

impl Protocol for TimestampableProtocol {
    fn name(&self) -> &str { "timestampable" }
    fn description(&self) -> &str { "创建和更新时自动注入时间戳" }
    fn aspects(&self) -> Vec<Arc<dyn Aspect>> { vec![Arc::new(TimestampableAspect)] }
    fn built_in(&self) -> bool { true }
}
```

## 11. AspectDispatch helper

**文件：** `src/services/aspect_dispatch.rs`

内置表 Service 层的轻量 dispatch helper，每个操作只需 3 行：

```rust
pub struct AspectDispatch<'a> {
    pub engine: &'a AspectEngine,
    pub pool: &'a Pool,
    pub table: &'a str,
    pub user_id: Option<&'a str>,
    pub tenant_id: Option<&'a str>,
}

impl AspectDispatch<'_> {
    pub async fn before_create(&self, record: Record) -> AppResult<()>;
    pub async fn after_create(&self, record: Record);
    pub async fn before_update(&self, old_record: Record, new_record: Record) -> AppResult<()>;
    pub async fn after_update(&self, new_record: Record);
    pub async fn before_delete(&self, record: Record) -> AppResult<()>;
    pub async fn after_delete(&self);
}

/// 创建一个 minimal Record 用于 dispatch（只含 id）
pub fn id_record(id: &str) -> Record;
```

**使用示例（Service 层）：**

```rust
use crate::services::aspect_dispatch::{AspectDispatch, id_record};

pub async fn create_post(engine: &AspectEngine, pool: &Pool, user_id: &str, ...) -> AppResult<Post> {
    let dispatch = AspectDispatch {
        engine, pool, table: "posts",
        user_id: Some(user_id), tenant_id: None,
    };
    dispatch.before_create(id_record(&id)).await?;
    // ... repository INSERT ...
    dispatch.after_create(id_record(&id)).await;
    Ok(post)
}
```

## 12. 系统集成

### 12.1 AppState

```rust
pub struct AppState {
    pub pool: Pool,
    pub config: Arc<AppConfig>,
    pub aspect_engine: Arc<AspectEngine>,
    pub protocol_registry: Arc<ProtocolRegistry>,
    // ... 其他字段
}
```

### 12.2 启动流程

```rust
pub async fn build_app_state(config: &AppConfig) -> anyhow::Result<AppState> {
    // 1. 创建 ProtocolRegistry，注册内置 Protocol
    let mut protocol_registry = ProtocolRegistry::new();
    protocol_registry.register(OwnableProtocol);
    protocol_registry.register(TimestampableProtocol);
    let protocol_registry = Arc::new(protocol_registry);

    // 2. 创建 AspectEngine，通过 ProtocolRegistry 注册所有 Aspect
    let aspect_engine = Arc::new(AspectEngine::new());
    protocol_registry.register_aspects_into(&aspect_engine);

    // 3. 日志输出
    tracing::info!(
        "aspect engine initialized with {} aspect(s), {} protocol(s)",
        aspect_engine.aspects().len(),
        protocol_registry.names().len()
    );

    // ...
}
```

### 12.3 Content Type 表集成

**handler.rs** — `do_create` / `do_update` / `do_delete` / `do_list` 接入 AspectEngine dispatch：

```rust
fn make_base_ctx(user_id: Option<&str>, pool: &Pool) -> BaseContext {
    BaseContext::new(
        user_id.map(|s| s.to_string()),
        "default".into(),
        crate::utils::tz::now_str(),
    ).with_pool(pool.clone())
}

fn make_base_ctx_anon(pool: &Pool) -> BaseContext {
    make_base_ctx(None, pool)
}
```

**migration.rs** — 无条件添加系统列：

```rust
// 无条件添加 created_at / updated_at / created_by / updated_by 列
// 避免系统列与用户自定义列重复（检查 user_col_names HashSet）
```

**repository.rs** — `build_column_names` 无条件包含系统列。

### 12.4 内置表集成

已覆盖的内置表：

| 内置表 | create | update | delete | 文件 |
|---|---|---|---|---|
| posts | ✅ | ✅ | ✅ | `src/services/post.rs` |
| pages | ✅ | ✅ | ✅ | `src/services/page.rs` |
| comments | ✅ | ✅ | ✅ | `src/services/comment.rs` |
| categories | ✅ | ✅ | ✅ | `src/services/post.rs` |
| tags | ✅ | — | ✅ | `src/services/post.rs` |
| reusable_blocks | ✅ | ✅ | ✅ | `src/services/page.rs` |

**调用方传入方式：**

- HTTP handler：`&state.aspect_engine`, `&state.pool`
- Tauri commands：`&state.0.aspect_engine`, `&state.0.pool`

### 12.5 Aspect 编排位置

**Aspect 编排放在 Handler 层（不放在 Repository 层）：**

```
handler 调 aspect.before → repo (纯数据操作) → aspect.after
```

Repository 层保持纯粹的数据操作，不包含横切面逻辑。

## 13. 与旧机制的关系

| 旧机制 | v2.0 替代 |
|---|---|
| `auto_fill: UserId` | **废弃**，由 OwnableAspect 替代 |
| `auto_fill: CurrentTimestamp` | **废弃**，由 TimestampableAspect 替代 |
| `timestamps: true` (ContentTypeSchema) | **废弃**，由 TimestampableAspect 替代 |
| `author_id` 字段 | **改为** `created_by` + `updated_by` |
| `Timestamps` 标志 | **完全移除**（schema/migration/repository/handler/tests） |
| `TargetMatcher::Custom` | **删除**，未实现且无使用场景 |
| `TargetMatcher::TablePattern` | **删除**，未实现 |
| `TargetMatcher::Routes` / `Events` | **删除**，未实现 |
| `TargetInfo` struct | **删除**，未实现 |

## 14. 已知限制（Phase 3 完善）

**内置表 Aspect 注入的值未回写到 Repository 的 SQL 参数：**

当前 dispatch 生效但 Aspect 注入的值（如 `created_by`）未被 typed Repository 的 `sqlx::query().bind()` 使用。Repository 层仍使用手写的字段绑定。

**解决方向：**
- 方案 A：Repository 的 SQL 参数从 dispatch 后的 record 中读取
- 方案 B：保持现状，系统列在 Repository SQL 中硬编码（当前做法）

## 15. 迁移计划

### Phase 1 — 基础设施 ✅ 已完成

1. `src/aspects.rs` — Aspect trait + Context 类型 + Advice + Extensions + Pointcut + SqlType
2. `src/aspects/engine.rs` — AspectEngine 注册/调度/匹配 + enable/disable
3. Content Type 表接入 AspectEngine

### Phase 2 — 扩展 Aspects ⏳ 待做

1. `versionable` Protocol — 版本快照
2. `cacheable` Protocol — 缓存
3. `soft_deletable` Protocol — 软删除

### Phase 3 — 内置表迁移 ✅ 已完成

1. 内置表 service 层接入 AspectEngine
2. `AspectDispatch` helper 减少重复代码

### Phase 4 — 插件 Aspect ⏳ 待做

1. 插件 manifest.toml 的 `[[aspect]]` 声明
2. JS/Lua/WASM Aspect 执行器
3. 性能隔离和错误降级

### Phase 5 — Access Layer 和 HTTP Layer ⏳ 待做

1. access_check / access_filter 集成
2. 替换现有 rule_engine 为 Access Aspect
3. HTTP Layer Aspect（统一 middleware 注册）

## 16. 文件结构

```
src/
  aspects.rs                              — Aspect trait, Context 类型, Advice, Extensions, Pointcut, SqlType, ColumnDef
  aspects/
    engine.rs                             — AspectEngine 注册/调度/匹配/enable/disable/Debug

  protocols.rs                            — Protocol trait + ProtocolRegistry
  protocols/
    ownable.rs                            — OwnableAspect + OwnableProtocol
    timestampable.rs                      — TimestampableAspect + TimestampableProtocol

  services/
    aspect_dispatch.rs                    — AspectDispatch helper + id_record
    post.rs                               — create/update/delete_post + category + tag 已接入 Aspect
    page.rs                               — create/update/delete_page + reusable_blocks 已接入 Aspect
    comment.rs                            — create/delete/update_status 已接入 Aspect

  content_type/
    handler.rs                            — do_create/do_update/do_delete/do_list 接入 AspectEngine
    migration.rs                          — 无条件系统列，避免与用户列重复
    repository.rs                         — build_column_names 无条件包含系统列
    schema.rs                             — 删除 timestamps 字段，新增 builtin/implements 字段

  lib.rs                                  — AppState 含 aspect_engine + protocol_registry, build_app_state 启动流程
```

## 17. 设计原则

1. **一次定义，全局生效** — 每个 Aspect 定义一次，对所有表自动生效
2. **声明式** — 通过 Protocol 组合 Aspect，通过 `implements` 或代码注册启用
3. **优先级驱动** — 明确的执行顺序，避免隐式依赖
4. **类型安全** — 每个 JoinPoint 有专属 Context 类型
5. **可扩展** — 未来插件可注册 JS/Lua/WASM Aspect
6. **事务感知** — Data 层在事务内，Event 层在事务外
7. **优雅降级** — after hook 失败只记 warn 日志，不阻断主操作
