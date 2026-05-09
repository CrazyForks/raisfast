# Workflow Engine 技术产品文档

> 版本：v1.0 · 最后更新：2026-05-09

## 目录

1. [概述](#1-概述)
2. [架构](#2-架构)
3. [数据模型](#3-数据模型)
4. [步骤类型详解](#4-步骤类型详解)
5. [API 参考](#5-api-参考)
6. [使用示例](#6-使用示例)
7. [当前限制](#7-当前限制)
8. [升级路线图](#8-升级路线图)

---

## 1. 概述

### 1.1 定位

raisfast 内置工作流引擎，为 CMS 内容发布流程提供自动化编排能力。设计目标：

- **轻量**：不依赖外部服务（无 Redis/MQ），数据全部存储在 SQLite 中
- **同步驱动**：步骤推进由 API 调用触发，而非后台轮询（除 Delay 类型）
- **声明式定义**：工作流以 JSON 定义步骤和转移关系，存储在 `workflow_definitions` 表
- **可观测**：每步执行生成 `workflow_step_logs` 记录，含输入/输出/耗时

### 1.2 设计原则

| 原则 | 说明 |
|------|------|
| 极简核心 | 5 种步骤类型覆盖 90% 常见场景 |
| 单实例单步骤 | 任意时刻一个实例只处于一个当前步骤（Parallel 除外） |
| Context 传递 | 工作流上下文在步骤间自动传递和合并 |
| 幂等安全 | 定义创建时校验所有引用完整性 |

### 1.3 不做的事

- 不做分布式协调（无 leader election、分布式锁）
- 不做 DAG 拓扑（无复杂依赖图）
- 不做可视化编辑器后端（前端自行实现）
- 不做跨系统编排（无 HTTP callback、无 gRPC 调用）

---

## 2. 架构

```
┌─────────────────────────────────────────────────┐
│                   HTTP API                       │
│  /admin/workflows       CRUD                     │
│  /admin/workflows/{id}/start                     │
│  /admin/workflows/instances/{id}/execute         │
│  /admin/workflows/instances/{id}/cancel          │
│  /admin/workflows/instances/{id}/logs            │
└────────────────────┬────────────────────────────┘
                     │
┌────────────────────▼────────────────────────────┐
│              WorkflowService                      │
│  ┌─────────────┐  ┌──────────────────────────┐  │
│  │ validate    │  │ execute_step              │  │
│  │  _steps     │  │  ├─ Task/Await/Delay      │  │
│  │             │  │  ├─ Branch (条件路由)      │  │
│  │ resolve     │  │  └─ Parallel (并行→汇合)  │  │
│  │  _next_step │  │                            │  │
│  └─────────────┘  └──────────────────────────┘  │
└────────────────────┬────────────────────────────┘
                     │
┌────────────────────▼────────────────────────────┐
│              Model 层 (sqlx)                     │
│  workflow_definitions                            │
│  workflow_instances                              │
│  workflow_step_logs                              │
└─────────────────────────────────────────────────┘
```

### 三层分离

| 层 | 文件 | 职责 |
|----|------|------|
| Handler | `src/handlers/workflow.rs` | HTTP 请求解析、参数校验、响应封装 |
| Service | `src/services/workflow.rs` | 状态机逻辑、步骤解析、条件评估、并行调度 |
| Model | `src/models/workflow.rs` | SQL 查询、数据结构映射 |

---

## 3. 数据模型

### 3.1 workflow_definitions — 工作流定义

| 字段 | 类型 | 说明 |
|------|------|------|
| `id` | INTEGER PK | 自增主键 |
| `document_id` | TEXT UNIQUE | UUID v7，对外暴露的 ID |
| `name` | TEXT | 工作流名称 |
| `description` | TEXT? | 描述 |
| `steps` | TEXT | JSON 数组，步骤定义（见 4.1） |
| `initial_step` | TEXT | 入口步骤 ID |
| `version` | INTEGER | 版本号（当前未使用，预留） |
| `enabled` | BOOLEAN | 是否启用，禁用时无法启动新实例 |
| `created_at` | TEXT | ISO 8601 |
| `updated_at` | TEXT | ISO 8601 |

### 3.2 workflow_instances — 工作流实例

| 字段 | 类型 | 说明 |
|------|------|------|
| `id` | INTEGER PK | 自增主键 |
| `document_id` | TEXT UNIQUE | UUID v7 |
| `definition_id` | INTEGER FK | 关联定义 |
| `status` | TEXT | `running` / `completed` / `failed` / `cancelled` |
| `current_step` | TEXT? | 当前步骤 ID（`completed`/`failed`/`cancelled` 时为 NULL） |
| `context` | TEXT | JSON，工作流上下文（跨步骤传递） |
| `triggered_by` | INTEGER? | 触发者 user.id |
| `started_at` | TEXT | 启动时间 |
| `completed_at` | TEXT? | 完成时间 |
| `updated_at` | TEXT | 最后更新 |

**实例状态机：**

```
                   start_workflow
                        │
                        ▼
                   ┌──────────┐
          ┌───────│ running  │───────┐
          │       └──────────┘       │
          │          │     │         │
    execute_step  cancel  fail_step  │
          │        │       │         │
          ▼        ▼       ▼         │
     ┌──────────┐  │  ┌─────────┐    │
     │ running  │  │  │ failed  │    │
     │(下一步)  │  │  └─────────┘    │
     └──────────┘  │                 │
          │        ▼                 │
          │  ┌───────────┐           │
          │  │ cancelled │           │
          │  └───────────┘           │
          ▼                          │
     ┌───────────┐                   │
     │ completed │◄──────────────────┘
     └───────────┘   (最后一步 next 为空)
```

### 3.3 workflow_step_logs — 步骤执行日志

| 字段 | 类型 | 说明 |
|------|------|------|
| `id` | INTEGER PK | 自增主键 |
| `document_id` | TEXT UNIQUE | UUID v7 |
| `instance_id` | INTEGER FK | 关联实例 |
| `step_id` | TEXT | 步骤定义 ID |
| `step_name` | TEXT | 步骤名称（冗余，方便查询） |
| `status` | TEXT | `running` / `completed` / `failed` |
| `input` | TEXT? | JSON，步骤开始时的 context 快照 |
| `output` | TEXT? | JSON，步骤完成时的输出 |
| `error` | TEXT? | 错误信息（仅 failed） |
| `started_at` | TEXT | 开始时间 |
| `completed_at` | TEXT? | 完成时间 |

---

## 4. 步骤类型详解

### 4.1 StepDef 结构

每个步骤定义包含以下字段：

```json
{
  "id": "review",
  "name": "审核",
  "type": "await",
  "config": {},
  "next": "publish",
  "timeout_ms": 0
}
```

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `id` | string | 是 | 步骤唯一标识，工作流内不可重复 |
| `name` | string | 是 | 显示名称 |
| `type` | string | 是 | 步骤类型：`task` / `await` / `branch` / `parallel` / `delay` |
| `config` | object | 否 | 步骤配置（各类型含义不同） |
| `next` | any | 否 | 下一步转移规则（各类型格式不同） |
| `timeout_ms` | number | 否 | 超时毫秒数（当前预留，未实现自动超时） |

### 4.2 Task — 自动任务

**语义**：系统自动执行的步骤。调用方通过 `execute_step` 提交结果后推进到 `next`。

**`next` 格式**：`string`，目标步骤 ID 或空字符串（表示结束）。

```json
{
  "id": "notify",
  "name": "发送通知",
  "type": "task",
  "config": { "channel": "email" },
  "next": "archive"
}
```

**执行流程**：

1. `execute_step` 被调用，`step_output` 合并到 context
2. 当前步骤 log 标记为 `completed`
3. `resolve_next_step` 返回 `next` 指定的步骤 ID
4. 实例推进到下一步

**`next` 特殊值**：

| 值 | 行为 |
|------|------|
| `"s2"` | 推进到步骤 `s2` |
| `""` / `null` | 工作流完成（status = `completed`） |

### 4.3 Await — 等待外部事件

**语义**：等待人工操作（如审批）或外部系统回调。与 Task 共享相同的状态转移逻辑，区别在于语义——Await 表示"需要外部触发"。

**`next` 格式**：与 Task 相同。

```json
{
  "id": "approve",
  "name": "等待审批",
  "type": "await",
  "config": { "roles": ["admin", "editor"] },
  "next": "publish"
}
```

**执行流程**：与 Task 完全一致。区分 `task` 和 `await` 的目的是让前端知道当前步骤是否需要用户交互。

### 4.4 Branch — 条件分支

**语义**：根据工作流 context 中的字段值选择下一步。

**`next` 格式**：`Array<{ condition?: Object, step: string }>`

- `condition`：键值对，所有字段必须完全匹配
- 无 `condition` 的分支是 fallback（当所有条件都不匹配时）
- 按数组顺序评估，第一个匹配的分支生效

```json
{
  "id": "decide",
  "name": "审核结果",
  "type": "branch",
  "config": {},
  "next": [
    { "condition": { "approved": true }, "step": "publish" },
    { "condition": { "approved": false, "reason": "reject" }, "step": "notify_reject" },
    { "step": "draft" }
  ]
}
```

**条件匹配规则**：

| 期望值类型 | 比较方式 |
|-----------|---------|
| String | `context[key].as_str() == expected` |
| Number | `context[key].as_f64() == expected.as_f64()` |
| Bool | `context[key].as_bool() == expected` |
| null / Object / Array | 不匹配，返回 `false` |

所有 condition 中的字段必须同时匹配（AND 逻辑）。缺少某个字段的 context 也不匹配。

### 4.5 Parallel — 并行执行

**语义**：同时启动多个分支，所有分支完成后汇合到 `join_next` 或结束。

**`next` 格式**：`string[]`（分支步骤 ID 数组）

**`config` 字段**：

| 字段 | 类型 | 说明 |
|------|------|------|
| `join_next` | `string?` | 所有分支完成后的下一步。省略则所有分支完成后直接结束工作流 |

```json
{
  "id": "parallel_notify",
  "name": "并行通知",
  "type": "parallel",
  "config": { "join_next": "generate_report" },
  "next": ["email_notify", "slack_notify"]
}
```

**执行流程**：

```
                 execute_step (Parallel)
                         │
           ┌─────────────┼─────────────┐
           │  创建所有分支的 running log  │
           │  context 写入 _parallel    │
           │  current_step = 分支[0]    │
           └─────────────┼─────────────┘
                         │
              ┌──────────▼──────────┐
              │   execute_step      │
              │   (分支 A - task)   │
              └──────────┬──────────┘
                         │ 完成 A，pending 移除 A
                         │ remaining > 0
                         │ current_step = 分支 B
              ┌──────────▼──────────┐
              │   execute_step      │
              │   (分支 B - task)   │
              └──────────┬──────────┘
                         │ 完成 B，pending 清空
                         │ remaining == 0
                         │ 清除 _parallel
                         │
                ┌────────▼────────┐
                │ join_next 存在？ │
                └───┬─────────┬───┘
                    │ Yes     │ No
                    ▼         ▼
             推进到       工作流
            join_next    completed
```

**并行状态追踪**：通过 context 中的 `_parallel` 字段实现（内部字段，API 不暴露）：

```json
{
  "_parallel": {
    "parent": "parallel_notify",
    "pending": ["slack_notify"],
    "join_next": "generate_report"
  }
}
```

**注意**：当前实现为**顺序执行**分支（串行通过 `execute_step` 逐一完成），而非真正的并发执行。这是基于以下考虑：

1. CMS 场景下"并行"通常是逻辑上的编排需求（"这三件事都需要做"），而非性能需求
2. SQLite 不支持高并发写入
3. 真正的并发需要后台 worker 支持

### 4.6 Delay — 延迟等待

**语义**：等待指定时间后自动推进。

**`next` 格式**：与 Task 相同。

```json
{
  "id": "wait_24h",
  "name": "等待24小时",
  "type": "delay",
  "config": { "duration_ms": 86400000 },
  "next": "check_status"
}
```

**当前状态**：`config` 中预留了 `duration_ms`，但引擎尚未实现自动超时推进。目前 Delay 步骤的行为与 Task 相同——需要外部 `execute_step` 调用来推进。

---

## 5. API 参考

所有 API 位于 `/api/v1/admin/workflows` 前缀下。

### 5.1 创建工作流定义

```
POST /api/v1/admin/workflows
```

**请求体**：

```json
{
  "id": "content-review",
  "name": "内容审核流程",
  "description": "文章发布前的多级审核",
  "steps": [
    { "id": "draft", "name": "起草", "type": "task", "config": {}, "next": "review" },
    { "id": "review", "name": "审核", "type": "await", "config": {}, "next": "" }
  ]
}
```

**响应**：

```json
{
  "code": 0,
  "message": "created",
  "data": { "id": 1, "document_id": "content-review", "name": "内容审核流程", ... }
}
```

**错误码**：

| 状态码 | 场景 |
|--------|------|
| 400 | steps 为空、步骤引用不存在的 ID、parallel/branch 的 next 格式错误 |

### 5.2 列出所有定义

```
GET /api/v1/admin/workflows
```

### 5.3 获取单个定义

```
GET /api/v1/admin/workflows/{id}
```

### 5.4 删除定义

```
DELETE /api/v1/admin/workflows/{id}
```

注意：删除定义不会级联删除已有实例。

### 5.5 启动工作流实例

```
POST /api/v1/admin/workflows/{id}/start
```

**请求体**：

```json
{
  "context": { "title": "新文章", "author": "张三" },
  "triggered_by": "user-doc-id-xxx"
}
```

**响应**：返回 `WorkflowInstance`，`status = "running"`，`current_step = initial_step`。

**错误码**：

| 状态码 | 场景 |
|--------|------|
| 400 | 工作流定义已禁用（`enabled = false`） |
| 404 | 工作流定义不存在 |

### 5.6 执行当前步骤

```
POST /api/v1/admin/workflows/instances/{instanceId}/execute
```

**请求体**：

```json
{
  "output": { "approved": true, "comment": "通过" }
}
```

**行为**：

1. `output` 中的键值合并到工作流 context
2. 当前步骤 log 标记为 `completed`
3. 根据步骤类型和 context 决定下一步

**错误码**：

| 状态码 | 场景 |
|--------|------|
| 400 | 实例不是 `running` 状态 |
| 404 | 实例不存在 |

### 5.7 取消工作流实例

```
POST /api/v1/admin/workflows/instances/{instanceId}/cancel
```

**错误码**：

| 状态码 | 场景 |
|--------|------|
| 400 | 实例不是 `running` 状态 |
| 404 | 实例不存在 |

### 5.8 获取步骤日志

```
GET /api/v1/admin/workflows/instances/{instanceId}/logs
```

**响应**：

```json
{
  "code": 0,
  "message": "ok",
  "data": [
    {
      "step_id": "draft",
      "step_name": "起草",
      "status": "completed",
      "input": "{\"title\":\"新文章\"}",
      "output": "{\"title\":\"新文章\",\"content\":\"...\"}",
      "started_at": "2026-05-09T10:00:00Z",
      "completed_at": "2026-05-09T10:05:00Z"
    }
  ]
}
```

### 5.9 列出工作流实例

```
GET /api/v1/admin/workflows/instances?definition_id=xxx&status=running&page=1&page_size=20
```

所有查询参数均可选。

---

## 6. 使用示例

### 6.1 简单审批流

```
起草 → 审核 → 发布
```

```json
{
  "id": "simple-review",
  "name": "简单审批",
  "steps": [
    { "id": "draft", "name": "起草", "type": "task", "config": {}, "next": "review" },
    { "id": "review", "name": "审核", "type": "await", "config": {}, "next": "publish" },
    { "id": "publish", "name": "发布", "type": "task", "config": {}, "next": "" }
  ]
}
```

### 6.2 条件分支流

```
起草 → 审核 → [通过?发布 : 驳回通知]
```

```json
{
  "id": "branch-review",
  "name": "分支审批",
  "steps": [
    { "id": "draft", "name": "起草", "type": "task", "config": {}, "next": "review" },
    {
      "id": "review",
      "name": "审核决策",
      "type": "branch",
      "config": {},
      "next": [
        { "condition": { "approved": true }, "step": "publish" },
        { "step": "reject" }
      ]
    },
    { "id": "publish", "name": "发布", "type": "task", "config": {}, "next": "" },
    { "id": "reject", "name": "驳回通知", "type": "task", "config": {}, "next": "" }
  ]
}
```

调用 `execute_step` 时传入 output：

```json
{ "output": { "approved": true } }
```

### 6.3 并行通知 + 汇合

```
           ┌→ 邮件通知 ─┐
提交审核 →  │            │ → 生成报告
           └→ Slack通知 ┘
```

```json
{
  "id": "parallel-notify",
  "name": "并行通知",
  "steps": [
    { "id": "submit", "name": "提交", "type": "task", "config": {}, "next": "notify" },
    {
      "id": "notify",
      "name": "并行通知",
      "type": "parallel",
      "config": { "join_next": "report" },
      "next": ["email", "slack"]
    },
    { "id": "email", "name": "邮件通知", "type": "task", "config": {}, "next": "" },
    { "id": "slack", "name": "Slack通知", "type": "task", "config": {}, "next": "" },
    { "id": "report", "name": "生成报告", "type": "task", "config": {}, "next": "" }
  ]
}
```

### 6.4 完整内容发布流

```json
{
  "id": "content-publish",
  "name": "内容发布流程",
  "description": "含审核、并行通知、条件发布的完整流程",
  "steps": [
    { "id": "draft", "name": "起草", "type": "task", "config": {}, "next": "review" },
    { "id": "review", "name": "编辑审核", "type": "await", "config": {}, "next": "decide" },
    {
      "id": "decide",
      "name": "审核结果",
      "type": "branch",
      "config": {},
      "next": [
        { "condition": { "approved": true, "priority": "high" }, "step": "urgent_publish" },
        { "condition": { "approved": true }, "step": "normal_publish" },
        { "step": "back_to_draft" }
      ]
    },
    { "id": "urgent_publish", "name": "紧急发布", "type": "task", "config": {}, "next": "notify" },
    { "id": "normal_publish", "name": "普通发布", "type": "task", "config": {}, "next": "" },
    {
      "id": "notify",
      "name": "通知相关人员",
      "type": "parallel",
      "config": { "join_next": "log" },
      "next": ["notify_author", "notify_editors"]
    },
    { "id": "notify_author", "name": "通知作者", "type": "task", "config": {}, "next": "" },
    { "id": "notify_editors", "name": "通知编辑组", "type": "task", "config": {}, "next": "" },
    { "id": "log", "name": "记录日志", "type": "task", "config": {}, "next": "" },
    { "id": "back_to_draft", "name": "退回修改", "type": "task", "config": {}, "next": "draft" }
  ]
}
```

---

## 7. 当前限制

### 7.1 已知限制

| 限制 | 说明 |
|------|------|
| Parallel 顺序执行 | 并行分支通过串行 `execute_step` 完成，非真正并发 |
| Delay 未实现自动超时 | 需外部调用 `execute_step` 推进，未接入后台定时器 |
| 条件表达式仅支持等值比较 | 不支持 `>`/`<`/`contains`/正则等复杂条件 |
| 定义不可修改 | 创建后 steps 不可变更（需删除重建） |
| 无版本管理 | `version` 字段预留但未使用 |
| 无重试机制 | 步骤失败后无法自动重试 |
| 无超时告警 | `timeout_ms` 字段预留但未实现 |
| Context 无类型约束 | 所有值都是 `serde_json::Value`，无 schema 校验 |
| 取消不清理并行状态 | 并行执行中取消实例，不会标记各分支 log 为 cancelled |

### 7.2 性能边界

- 单实例最大步骤数：无硬限制（实际受 SQLite TEXT 列大小限制）
- 并行分支数：无硬限制（每个分支创建独立 step_log 行）
- Context 大小：受 SQLite TEXT 列限制（默认 1GB）
- 无分页的 `list_workflows`：定义数量大时需加分页

---

## 8. 升级路线图

按优先级分为 **P0（必需）**、**P1（重要）**、**P2（增强）** 三个等级。

### P0：核心补全

#### 8.1 Delay 自动超时

**目标**：Delay 步骤到时间后自动推进，无需外部调用。

**方案**：
- `WorkflowService::start_workflow` 检查初始步骤是否为 Delay，若是则注册定时器
- 使用 `tokio::time::sleep` + `tokio::spawn` 在后台到期后调用 `execute_step`
- Worker 调度器（`src/worker/scheduler.rs`）已有定时任务框架，可复用

**预估工作量**：1-2 天

#### 8.2 步骤超时检测

**目标**：`timeout_ms > 0` 的步骤在指定时间未完成时自动标记失败。

**方案**：
- 启动步骤时记录 `started_at`
- Worker 定时扫描 `workflow_step_logs WHERE status = 'running' AND (now - started_at) > timeout`
- 自动调用 `fail_step`

**预估工作量**：1 天

#### 8.3 定义更新

**目标**：支持修改已有工作流定义的 steps，不影响运行中的实例。

**方案**：
- 新增 `PUT /admin/workflows/{id}` 端点
- 运行中的实例继续使用启动时的 steps 快照（存储在 instance 上或首次加载时缓存）
- 新启动的实例使用最新定义

**预估工作量**：1 天

### P1：实用功能

#### 8.4 条件表达式增强

**目标**：支持比较运算符和嵌套字段。

**方案**：
```json
{
  "condition": {
    "score": { "$gt": 80 },
    "tags": { "$contains": "rust" },
    "meta.region": { "$in": ["us", "eu"] }
  }
}
```

参考 MongoDB 查询语法，实现 `$gt`/`$lt`/`$gte`/`$lte`/`$in`/`$contains`/`$regex` 运算符。

**预估工作量**：2-3 天

#### 8.5 步骤重试

**目标**：失败步骤可配置自动重试。

**方案**：
- StepDef 新增 `config.retry`：
  ```json
  { "retry": { "max_attempts": 3, "interval_ms": 5000, "backoff": "exponential" } }
  ```
- `fail_step` 检查重试配置，若未达上限则重新创建 running log
- 指数退避：`interval_ms * 2^(attempt-1)`

**预估工作量**：2 天

#### 8.6 Parallel 真正并发

**目标**：并行分支真正同时执行，而非串行。

**方案**：
- 使用 `tokio::spawn` 并发执行各分支
- 分支结果通过 `JoinHandle` 收集
- 所有分支完成后才推进到 `join_next`

**前提**：需要先实现真正的后台 Task 执行能力（见 8.8）。

**预估工作量**：3-5 天

#### 8.7 工作流事件

**目标**：工作流状态变更时发出事件，供 Webhook/通知系统消费。

**方案**：
- 复用现有 `EventBus`，新增事件类型：
  - `WorkflowStarted { instance_id, definition_id }`
  - `StepCompleted { instance_id, step_id, output }`
  - `StepFailed { instance_id, step_id, error }`
  - `WorkflowCompleted { instance_id }`
  - `WorkflowCancelled { instance_id }`
- Handler 层在调用 service 后 emit 事件

**预估工作量**：1-2 天

### P2：高级特性

#### 8.8 Task 自动执行

**目标**：Task 步骤可绑定 plugin hook 或内置 action，自动执行而非等待外部 `execute_step`。

**方案**：
- StepDef config 新增 `action`：
  ```json
  { "action": "plugin:email/send", "params": { "to": "{{context.author_email}}" } }
  ```
- 引擎在推进到 Task 步骤时自动调用对应 handler
- 支持 `mustache` 模板语法从 context 中取值

**预估工作量**：5-7 天

#### 8.9 子工作流

**目标**：步骤可嵌套调用另一个工作流。

**方案**：
- 新增步骤类型 `SubWorkflow`：
  ```json
  { "id": "sub", "type": "sub_workflow", "config": { "workflow_id": "approval-flow" }, "next": "..." }
  ```
- 父实例等待子实例完成后获取其 output 作为自身 step output

**预估工作量**：3-5 天

#### 8.10 可视化定义

**目标**：前端拖拽式工作流编辑器 + 后端验证。

**方案**：
- 前端使用 React Flow / dagre.js 绘制流程图
- 导出为现有 JSON 格式
- 后端新增 `POST /admin/workflows/validate` 校验端点

**预估工作量**：前端为主，后端 1 天

#### 8.11 版本管理

**目标**：工作流定义支持版本化和灰度发布。

**方案**：
- `workflow_definitions` 新增 `version` 自增逻辑
- 实例记录 `definition_version`
- 支持回滚到指定版本

**预估工作量**：2-3 天

#### 8.12 审计与监控

**目标**：集成 `audit_log`，提供工作流运行统计面板。

**方案**：
- 复用 `src/audit.rs`，记录关键操作
- 新增统计 API：
  - `GET /admin/workflows/stats` — 定义数、运行中/完成/失败实例数
  - `GET /admin/workflows/instances/{id}/timeline` — 甘特图数据
- 平均步骤耗时计算

**预估工作量**：2-3 天

---

## 升级优先级总览

| 优先级 | 编号 | 名称 | 工作量 | 价值 |
|--------|------|------|--------|------|
| **P0** | 8.1 | Delay 自动超时 | 1-2 天 | 完善 Delay 语义 |
| **P0** | 8.2 | 步骤超时检测 | 1 天 | 防止步骤永久卡住 |
| **P0** | 8.3 | 定义更新 | 1 天 | 基础管理能力 |
| **P1** | 8.4 | 条件表达式增强 | 2-3 天 | 解锁复杂业务场景 |
| **P1** | 8.5 | 步骤重试 | 2 天 | 提高可靠性 |
| **P1** | 8.6 | Parallel 真正并发 | 3-5 天 | 性能提升 |
| **P1** | 8.7 | 工作流事件 | 1-2 天 | 可观测性 |
| **P2** | 8.8 | Task 自动执行 | 5-7 天 | 真正的自动化 |
| **P2** | 8.9 | 子工作流 | 3-5 天 | 流程复用 |
| **P2** | 8.10 | 可视化定义 | 后端 1 天 | 用户体验 |
| **P2** | 8.11 | 版本管理 | 2-3 天 | 安全变更 |
| **P2** | 8.12 | 审计与监控 | 2-3 天 | 运维能力 |

**建议实施顺序**：8.3 → 8.1 → 8.2 → 8.7 → 8.4 → 8.5 → 8.12 → 8.8 → 8.6 → 8.11 → 8.9 → 8.10
