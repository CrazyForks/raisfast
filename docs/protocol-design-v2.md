# AOP + Protocol 设计文档 — v2.0 实现版

> 版本：v2.0 | 最后更新：2026-05-07
> 状态：Data Layer 已实现，11 个内置协议全部完成

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
  ├─ Aspect before_create       ← Aspect 引擎 dispatch
  ├─ Aspect after_create        ← Aspect 引擎 dispatch
  │
Repository 层
  ├─ 协议声明驱动 SQL           ← ProtocolDeclaration
  ├─ INSERT/UPDATE/DELETE SQL
  │
EventBus 层
  ├─ 审计日志                   ← subscriber
  ├─ Webhook 通知               ← subscriber
  └─ 搜索索引                   ← 未实现
```

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
│  ProtocolDeclaration  协议的声明式效果（纯数据）           │
│  Layer       Aspect 关心的系统层级                         │
│  Pointcut    Aspect 关心的拦截点（精确匹配或模式匹配）      │
│  Advice      Aspect 在拦截点上的具体行为                    │
│  Context     拦截点的上下文数据（可读/可写）                 │
│  Engine      调度器，管理 Aspect 注册和执行                  │
└──────────────────────────────────────────────────────────┘
```

### 两个正交维度

每个协议将行为拆分为：

| 维度 | 实现方式 | 示例 |
|------|----------|------|
| **命令式**（注入值） | Aspect + Pointcut | `on_data_before_create` 注入 `created_at` |
| **声明式**（影响 SQL 行为） | `ProtocolDeclaration` 纯数据 | `query_filters` → WHERE、`default_sort` → ORDER BY |

## 3. 三层架构

```
┌─ aspects.rs + aspects/ ──────────────────────────────────┐
│  纯框架层，不知道 Protocol 存在                          │
│  Aspect trait + AspectEngine + Context 类型 + Advice    │
└─────────────────────────────────────────────────────────┘
          ▲
          │ register_from_arc()
┌─ protocols.rs + protocols/ ─────────────────────────────┐
│  业务 Protocol 实现（1:N 组合 Aspect）                   │
│  Protocol trait + ProtocolRegistry + ProtocolDeclaration │
│  ownable.rs / timestampable.rs / ... (11 个)             │
└─────────────────────────────────────────────────────────┘
          ▲
          │ register_from_inventory()
┌─ lib.rs ────────────────────────────────────────────────┐
│  protocol_registry.register_from_inventory() 一行注册    │
│  protocol_registry.register_aspects_into(&engine)       │
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
│  适用：字段注入、校验、版本快照、搜索索引          │
│  状态：已实现 ✅                                        │
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

### 5.3 TargetMatcher

```rust
#[derive(Debug, Clone)]
pub enum TargetMatcher {
    /// 匹配所有目标
    All,
    /// 精确匹配表名列表
    Tables(Vec<String>),
}
```

### 5.4 Pointcut

```rust
#[derive(Debug, Clone)]
pub struct Pointcut {
    pub layer: Layer,
    pub operation: Operation,
    pub when: When,
    pub target: TargetMatcher,
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
    pub fn as_str(self) -> &'static str {
        match self {
            SqlType::Text => { /* SQLite: TEXT / PostgreSQL: TEXT / MySQL: VARCHAR(255) */ }
            SqlType::Integer => { /* SQLite: INTEGER / PostgreSQL: INTEGER / MySQL: INT */ }
            SqlType::BigInt => { /* SQLite: INTEGER / PostgreSQL: BIGINT / MySQL: BIGINT */ }
            SqlType::Real => { /* SQLite: REAL / PostgreSQL: DOUBLE PRECISION / MySQL: DOUBLE */ }
            SqlType::Boolean => { /* SQLite: BOOLEAN / PostgreSQL: BOOLEAN / MySQL: TINYINT(1) */ }
            SqlType::Blob => { /* SQLite: BLOB / PostgreSQL: BYTEA / MySQL: BLOB */ }
        }
    }
}
```

### 5.7 ColumnDef

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnDef {
    pub name: String,
    pub sql_type: SqlType,
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
    pub pool: Option<crate::db::pool::Pool>,
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

### 8.6 优先级约定

```
优先级范围        用途                           示例
─────────────────────────────────────────────────────────
 -9999 ~ -1000   核心基础设施                   tenantable (-600)
  -999 ~ -500    数据注入                       ownable (-500), timestampable (-400)
  -499 ~    0    默认
     1 ~  499    业务逻辑                       validation, slug 生成
   500 ~  999    副作用（事务内）                版本快照、搜索索引
  1000 ~  9999   事后处理（事务内）              缓存清除
 10000+          非关键（可降级）                统计、监控
```

## 9. Protocol 层

### 9.1 概念

Protocol = Aspect 组合 + 声明式效果（ProtocolDeclaration）。

```
ContentTypeSchema.implements = ["ownable", "timestampable", "soft_deletable"]
                         │
                         ▼
ProtocolRegistry.get("ownable")
  ├─ name: "ownable"
  ├─ description: "创建和更新时自动注入操作者 ID"
  ├─ aspects: [OwnableAspect]
  ├─ columns: [created_by, updated_by]
  ├─ behaviors: ["track_owner"]
  ├─ declaration() → ProtocolDeclaration { ... }
  ├─ apply_config() → 用户配置应用到声明
  ├─ register_routes() → 协议自注册 API 路由
  └─ on_after_delete() → 异步清理 hook
```

### 9.2 Protocol trait

```rust
pub trait Protocol: Send + Sync + 'static {
    fn name(&self) -> &str;
    fn description(&self) -> &str { "" }
    fn aspects(&self) -> Vec<Arc<dyn Aspect>>;
    fn columns(&self) -> Vec<ColumnDef> {
        self.aspects().iter().flat_map(|a| a.columns()).collect()
    }
    fn behaviors(&self) -> Vec<&'static str> { vec![] }
    fn built_in(&self) -> bool { false }

    /// 协议声明式效果（纯数据）
    fn declaration(&self) -> ProtocolDeclaration {
        ProtocolDeclaration::default()
    }

    /// 将用户配置应用到声明（如 sortable 的 field/direction）
    fn apply_config(&self, _config: &HashMap<String, String>, _decl: &mut ProtocolDeclaration, _all_columns: &[&str]) {}

    /// 注册协议所需的额外 API 路由
    fn register_routes(&self, _router: Router<AppState>, _plural: &str, _admin_prefix: &str) -> Router<AppState> { _router }

    /// 删除记录后的异步回调
    fn on_after_delete(&self, _pool: &Pool, _singular: &str, _id: &str) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>> { Box::pin(async { Ok(()) }) }
}
```

### 9.3 ProtocolDeclaration

```rust
#[derive(Debug, Clone, Default)]
pub struct ProtocolDeclaration {
    /// 查询时自动追加的 WHERE 过滤条件: (column, SQL_condition)
    pub query_filters: Vec<(String, String)>,
    /// 删除策略
    pub delete_strategy: DeleteStrategy,
    /// 更新前是否获取当前记录快照
    pub snapshot_before_update: bool,
    /// 是否提供版本历史 API 路由
    pub revision_routes: bool,
    /// 乐观锁列名
    pub lock_column: Option<String>,
    /// 列表查询的默认排序 (column, direction)
    pub default_sort: Option<(String, SortDir)>,
    /// statusable: 允许的状态值列表
    pub status_values: Option<Vec<String>>,
    /// statusable: 数字映射 (label → number)
    pub status_map: Option<Vec<(String, i64)>>,
    /// statusable: 默认状态值
    pub status_default: Option<String>,
    /// statusable: 存储模式
    pub status_mode: StatusMode,
}
```

### 9.4 merge() 策略

多个协议的 `ProtocolDeclaration` 通过 `merge()` 聚合：

| 字段 | 策略 |
|------|------|
| `query_filters` | 累积（extend） |
| `delete_strategy` | Soft 优先 Hard |
| `snapshot_before_update` / `revision_routes` | OR |
| `lock_column` / `default_sort` | last-wins + warn on conflict |
| `status_*` | last-wins |

### 9.5 ProtocolRef

`implements` 字段支持两种语法：

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ProtocolRef {
    Simple(String),
    WithConfig {
        name: String,
        #[serde(flatten)]
        config: HashMap<String, String>,
    },
}
```

TOML 中使用：

```toml
# 简单语法
implements = ["ownable", "timestampable"]

# 带配置语法
implements = [
  { name = "sortable", field = "priority", direction = "desc" },
  { name = "statusable", values = "draft=1,published=10", default = "1", mode = "numeric" },
]
```

### 9.6 协议自注册（inventory）

每个协议文件末尾一行自注册：

```rust
// src/protocols/sortable.rs 末尾
crate::register_protocol!(
    crate::protocols::sortable::SortableProtocol,
    crate::protocols::sortable::SortableProtocol
);
```

`lib.rs` 中一行完成所有注册：

```rust
protocol_registry.register_from_inventory();
```

### 9.7 列冲突检测

`columns_for()` 对同名不同类型的列采用 **first-wins + warn**（不阻断启动），因为 sortable 可能用已有列排序。

### 9.8 ProtocolRegistry

```rust
pub struct ProtocolRegistry {
    protocols: HashMap<String, Arc<dyn Protocol>>,
}

impl ProtocolRegistry {
    pub fn new() -> Self;
    pub fn register(&mut self, protocol: impl Protocol);
    pub fn register_from_inventory(&mut self);
    pub fn get(&self, name: &str) -> Option<&Arc<dyn Protocol>>;
    pub fn names(&self) -> Vec<&str>;

    /// 按名称查询列（自动去重，first-wins + warn）
    pub fn columns_for(&self, names: &[String]) -> Vec<ColumnDef>;

    /// 按名称查询 Aspect（自动去重）
    pub fn aspects_for(&self, names: &[String]) -> Vec<Arc<dyn Aspect>>;

    /// 聚合多个协议的声明
    pub fn declaration_for(&self, names: &[String]) -> ProtocolDeclaration;

    /// 对聚合后的声明应用用户配置
    pub fn apply_config_for(&self, impl_refs: &[ProtocolRef], decl: &mut ProtocolDeclaration, all_columns: &[&str]);

    /// 注册所有协议的额外路由
    pub fn register_routes_for(&self, names: &[String], router: Router<AppState>, plural: &str, admin_prefix: &str) -> Router<AppState>;

    /// 删除后回调
    pub async fn dispatch_after_delete(&self, names: &[String], pool: &Pool, singular: &str, id: &str) -> Result<()>;

    /// 将所有 Protocol 的 Aspect 注册到 AspectEngine（去重）
    pub fn register_aspects_into(&self, engine: &AspectEngine);
}
```

## 10. 内置 Protocol 实现（11 种）

### 10.1 ownable

**文件：** `src/protocols/ownable.rs`

| 属性 | 值 |
|------|------|
| Aspect | `OwnableAspect` (priority: -500) |
| 注入列 | `created_by` TEXT, `updated_by` TEXT |
| behaviors | `["track_owner"]` |
| Pointcuts | Data Create Before + Data Update Before |

Aspect 使用 `is_protocol_column` 守卫：
```rust
ctx.schema.as_ref().is_none_or(|s| s.is_protocol_column(COL_CREATED_BY))
```

### 10.2 timestampable

**文件：** `src/protocols/timestampable.rs`

| 属性 | 值 |
|------|------|
| Aspect | `TimestampableAspect` (priority: -400) |
| 注入列 | `created_at` TEXT, `updated_at` TEXT |
| behaviors | `["track_timestamps"]` |
| Pointcuts | Data Create Before + Data Update Before |

### 10.3 soft_deletable

**文件：** `src/protocols/soft_deletable.rs`

| 属性 | 值 |
|------|------|
| Aspect | `SoftDeletableAspect` |
| 注入列 | `deleted_at` TEXT, `deleted_by` TEXT |
| behaviors | `["soft_delete"]` |
| Declaration | `query_filters: [("deleted_at", "IS NULL")]`, `delete_strategy: Soft { column: "deleted_at" }` |

### 10.4 versionable

**文件：** `src/protocols/versionable.rs`

| 属性 | 值 |
|------|------|
| Aspect | `VersionableAspect` |
| 注入列 | `version` INTEGER |
| behaviors | `["versioning"]` |
| Declaration | `snapshot_before_update: true`, `revision_routes: true` |
| register_routes | `/revisions`, `/revisions/{rev_id}`, `/revisions/{rev_id}/restore`, `/revisions/{rev_a}/diff/{rev_b}` |
| on_after_delete | 清理 `content_revisions` 表中关联记录 |

### 10.5 lockable

**文件：** `src/protocols/lockable.rs`

| 属性 | 值 |
|------|------|
| 注入列 | `lock_version` INTEGER (DEFAULT 0) |
| behaviors | `["optimistic_lock"]` |
| Declaration | `lock_column: Some("lock_version")` |
| Repository 行为 | UPDATE WHERE lock_version = ? + SET lock_version += 1，冲突返回 409 |

### 10.6 sortable

**文件：** `src/protocols/sortable.rs`

| 属性 | 值 |
|------|------|
| 注入列 | 无（使用已有列排序） |
| behaviors | `["sortable"]` |
| Declaration | `default_sort: Some(("created_at", Desc))` |
| apply_config | 支持 `field` / `direction` 配置 |

```toml
implements = [{ name = "sortable", field = "priority", direction = "desc" }]
```

### 10.7 expirable

**文件：** `src/protocols/expirable.rs`

| 属性 | 值 |
|------|------|
| Aspect | `ExpirableAspect` (priority: -200) |
| 注入列 | `expires_at` TEXT |
| behaviors | `["expirable"]` |
| Declaration | `query_filters: [("expires_at", "IS NULL OR expires_at > datetime('now')")]` |

### 10.8 nestable

**文件：** `src/protocols/nestable.rs`

| 属性 | 值 |
|------|------|
| 注入列 | `parent_id` TEXT, `depth` INTEGER, `position` INTEGER |
| behaviors | `["nestable"]` |

### 10.9 statusable

**文件：** `src/protocols/statusable.rs`

| 属性 | 值 |
|------|------|
| Aspect | `StatusableAspect` (priority: -150) |
| 注入列 | `status` TEXT |
| behaviors | `["statusable"]` |
| apply_config | 支持 `values` / `default` / `mode` 配置 |
| Aspect 行为 | create 注入默认值 + create/update 校验合法值 |
| 存储模式 | 字符串模式（默认）或数字映射模式 |

```toml
implements = [{ name = "statusable", values = "draft=1,published=10,archived=99", default = "1", mode = "numeric" }]
```

> 查询过滤不由协议处理，由 API rule engine 控制：`[api.list] filter = 'status = "published"'`。

### 10.10 metaable

**文件：** `src/protocols/metaable.rs`

| 属性 | 值 |
|------|------|
| Aspect | `MetaableAspect` |
| 注入列 | `__meta` TEXT (DEFAULT '{}') |
| behaviors | `["metaable"]` |
| 查询支持 | `?__meta.views=100` → `json_extract(__meta, '$.views') = '100'` |

> `__meta` 不再硬编码到所有表，由 `implements = ["metaable"]` 控制。

### 10.11 tenantable

**文件：** `src/protocols/tenantable.rs`

| 属性 | 值 |
|------|------|
| 注入列 | `tenant_id` TEXT (NOT NULL DEFAULT 'default') |
| behaviors | `["tenantable"]` |
| Repository 行为 | `ct.implements_protocol("tenantable")` 判断，自动注入和过滤 |
| Aspect | 纯列声明（pointcuts 为空），tenant_id 值由 Repository 统一注入 |

> `tenant_id` 不再硬编码到所有表，由 `implements = ["tenantable"]` 控制。不再运行时检测 DB 列，改为 schema 级别判断。

## 11. Aspect `is_protocol_column` 守卫

所有 Aspect 的 `on_data_before_create` / `on_data_before_update` 统一使用：

```rust
ctx.schema.as_ref().is_none_or(|s| s.is_protocol_column(COL_XXX))
```

- 有 schema → 只在该协议列真正需要创建时注入
- 无 schema → 放行（向后兼容单元测试）

## 12. ProtocolDeclaration 消费方

Repository 层直接读取 `ct.declaration()` 获取聚合后的声明：

| 消费方 | 使用的字段 |
|--------|-----------|
| `find()` (列表查询) | `query_filters` → WHERE, `default_sort` → ORDER BY |
| `create()` | `tenant_id` 由 `ct.implements_protocol("tenantable")` 判断 |
| `update()` | `snapshot_before_update` → 保存快照, `lock_column` → 乐观锁 |
| `delete()` | `delete_strategy` → Soft/Hard 判断 |
| `soft_delete()` | `delete_strategy` → 软删除列名 |
| migration | `columns_for()` → 动态列 |
| handler | `is_soft_delete()` / `has_revision_routes()` / `declaration()` |

## 13. AspectDispatch helper

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

pub fn id_record(id: &str) -> Record;
```

## 14. 系统集成

### 14.1 AppState

```rust
pub struct AppState {
    pub pool: Pool,
    pub config: Arc<AppConfig>,
    pub aspect_engine: Arc<AspectEngine>,
    pub protocol_registry: Arc<ProtocolRegistry>,
    // ... 其他字段
}
```

### 14.2 启动流程

```rust
pub async fn build_app_state(config: &AppConfig) -> anyhow::Result<AppState> {
    // 1. 创建 ProtocolRegistry，inventory 自注册所有内置 Protocol
    let mut protocol_registry = ProtocolRegistry::new();
    protocol_registry.register_from_inventory();
    let protocol_registry = Arc::new(protocol_registry);

    // 2. 创建 AspectEngine，通过 ProtocolRegistry 注册所有 Aspect
    let aspect_engine = Arc::new(AspectEngine::new());
    protocol_registry.register_aspects_into(&aspect_engine);
}
```

### 14.3 Content Type Handler 集成

handler.rs — `do_create` / `do_update` / `do_delete` / `do_list` / `do_get` 接入 AspectEngine dispatch。

Aspect 编排放在 Handler 层（不放在 Repository 层）：

```
handler 调 aspect.before → repo (纯数据操作) → aspect.after
```

Repository 层保持纯粹的数据操作，不包含横切面逻辑。

### 14.4 Content Type Migration 集成

migration.rs — 协议列由 `ProtocolRegistry::columns_for()` 动态获取，不再硬编码任何系统列。

### 14.5 Content Type Repository 集成

repository.rs — 基于 `ct.declaration()` 驱动 SQL 行为：
- `resolve_tenant()` — `ct.implements_protocol("tenantable")` 判断（同步，零 IO）
- `build_order_by()` — `declaration().default_sort`
- `find()` — `query_filters` + `meta_filters` (json_extract)
- `update()` — `lock_column` + `snapshot_before_update`
- `delete()` — `delete_strategy`

### 14.6 内置表集成

已覆盖的内置表：

| 内置表 | create | update | delete | 文件 |
|---|---|---|---|---|
| posts | ✅ | ✅ | ✅ | `src/services/post.rs` |
| pages | ✅ | ✅ | ✅ | `src/services/page.rs` |
| comments | ✅ | ✅ | ✅ | `src/services/comment.rs` |
| categories | ✅ | ✅ | ✅ | `src/services/post.rs` |
| tags | ✅ | — | ✅ | `src/services/post.rs` |
| reusable_blocks | ✅ | ✅ | ✅ | `src/services/page.rs` |

## 15. 与旧机制的关系

| 旧机制 | 当前替代 |
|---|---|
| `auto_fill: UserId` | **废弃**，由 OwnableAspect 替代 |
| `auto_fill: CurrentTimestamp` | **废弃**，由 TimestampableAspect 替代 |
| `timestamps: true` (ContentTypeSchema) | **废弃**，由 timestampable 协议替代 |
| `draft_publish` 字段 | **废弃**，由 statusable 协议替代 |
| `list_view` 配置 | **废弃**，由 sortable 协议的 default_sort 替代 |
| `cacheable` 协议 | **删除**，缓存由 handler 内置 DashMap TTL 处理 |
| `__meta` 硬编码 | **废弃**，由 metaable 协议控制 |
| `tenant_id` 硬编码 | **废弃**，由 tenantable 协议控制 |
| `has_tenant_id()` DB 检测 | **废弃**，改为 `ct.implements_protocol("tenantable")` |

## 16. 迁移计划

### Phase 1 — 基础设施 ✅ 已完成

1. `src/aspects.rs` — Aspect trait + Context 类型 + Advice + Extensions + Pointcut + SqlType
2. `src/aspects/engine.rs` — AspectEngine 注册/调度/匹配 + enable/disable
3. Content Type 表接入 AspectEngine

### Phase 2 — 扩展 Protocols ✅ 已完成

11 个内置协议全部实现：ownable、timestampable、soft_deletable、versionable、lockable、sortable、expirable、nestable、statusable、metaable、tenantable。

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

## 17. 文件结构

```
src/
  aspects.rs                              — Aspect trait, Context 类型, Advice, Extensions, Pointcut, SqlType, ColumnDef
  aspects/
    engine.rs                             — AspectEngine 注册/调度/匹配/enable/disable/Debug

  protocols.rs                            — Protocol trait + ProtocolRegistry + ProtocolDeclaration + DeleteStrategy + SortDir + StatusMode + inventory 自注册
  protocols/
    ownable.rs                            — OwnableAspect + OwnableProtocol
    timestampable.rs                      — TimestampableAspect + TimestampableProtocol
    soft_deletable.rs                     — SoftDeletableAspect + SoftDeletableProtocol
    versionable.rs                        — VersionableAspect + VersionableProtocol (register_routes + on_after_delete)
    lockable.rs                           — LockableProtocol
    sortable.rs                           — SortableProtocol (apply_config)
    expirable.rs                          — ExpirableAspect + ExpirableProtocol
    nestable.rs                           — NestableProtocol
    statusable.rs                         — StatusableAspect + StatusableProtocol (apply_config)
    metaable.rs                           — MetaableAspect + MetaableProtocol
    tenantable.rs                         — TenantableProtocol (纯列声明)

  constants.rs                            — COL_CREATED_BY, COL_UPDATED_BY, COL_CREATED_AT, COL_UPDATED_AT, COL_DELETED_AT, COL_DELETED_BY, COL_VERSION, COL_LOCK_VERSION, COL_SORT_KEY, COL_STATUS, COL_EXPIRES_AT, COL_PARENT_ID, COL_DEPTH, COL_POSITION, COL_META, COL_TENANT_ID, COL_ID

  services/
    aspect_dispatch.rs                    — AspectDispatch helper + id_record

  content_type/
    handler.rs                            — do_create/do_update/do_delete/do_list 接入 AspectEngine + DashMap 缓存 + meta_filters
    migration.rs                          — 协议列动态注入（不硬编码任何系统列）
    repository.rs                         — ProtocolDeclaration 驱动 SQL + schema 级 tenantable 判断
    schema.rs                             — ContentTypeSchema + ProtocolRef (Simple/WithConfig) + implements_protocol()

  lib.rs                                  — protocol_registry.register_from_inventory() + register_aspects_into()
```

## 18. 设计原则

1. **一次定义，全局生效** — 每个 Aspect 定义一次，对所有表自动生效
2. **声明式 + 命令式分离** — ProtocolDeclaration 纯数据驱动 SQL 行为，Aspect 处理命令式副作用
3. **协议组合** — 多个协议通过 `merge()` 聚合，first-wins/last-wins 策略明确
4. **优先级驱动** — 明确的执行顺序，避免隐式依赖
5. **类型安全** — 每个 JoinPoint 有专属 Context 类型
6. **可扩展** — 新协议只需 1 个文件 + 1 行 `register_protocol!`
7. **事务感知** — Data 层在事务内，Event 层在事务外
8. **优雅降级** — after hook 失败只记 warn 日志，不阻断主操作
9. **数据驱动** — 扩展 ProtocolDeclaration 字段时，`merge_covers_all_declaration_fields` 测试会红灯提醒
