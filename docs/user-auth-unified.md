# 统一用户认证系统设计

> raisfast 通用后端平台 — 用户注册/登录/绑定/解绑重构方案。
> 替代当前"email NOT NULL + 哨兵值"的打补丁方案。

---

## 1. 设计原则

| 原则 | 说明 |
|------|------|
| **账户与凭证分离** | `users` 表只存 profile，登录凭证独立存 `user_credentials` |
| **无哨兵值** | 不用 `!sms:`、`!oauth:` 之类的假数据填充 NOT NULL 列 |
| **可扩展** | 新增登录方式只需加一个 `auth_type` 枚举值，不改表结构 |
| **多凭证共存** | 一个用户可以同时绑定邮箱密码、手机号、多个 OAuth provider |
| **安全解绑** | 至少保留一个有效凭证，防止用户锁死自己 |

---

## 2. 表结构变更

### 2.1 `users` 表（精简 — 只保留 profile）

```sql
CREATE TABLE IF NOT EXISTS users (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    document_id TEXT NOT NULL UNIQUE,       -- UUID v7，对外 ID
    tenant_id TEXT NOT NULL DEFAULT 'default',
    username TEXT UNIQUE NOT NULL,          -- 唯一用户名
    display_name TEXT,
    avatar TEXT,
    bio TEXT,
    website TEXT,
    slug TEXT UNIQUE,
    locale TEXT,
    role TEXT NOT NULL DEFAULT 'reader',
    status TEXT NOT NULL DEFAULT 'active',  -- active / suspended / deleted
    registered_via TEXT NOT NULL,           -- 'email' | 'phone' | 'oauth_github' | ...  注册时写入，不再变更
    -- 移除: email, password_hash, phone, email_verified
    -- 这些全部迁移到 user_credentials 表
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_users_slug ON users(slug) WHERE slug IS NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS idx_users_username ON users(username);
```

**移除的字段：**

| 字段 | 原因 | 去向 |
|------|------|------|
| `email` | 不是所有用户都有 email | `user_credentials.identifier`（auth_type=email） |
| `password_hash` | 不是所有用户都有密码 | `user_credentials.credential_data`（auth_type=email） |
| `phone` | 不是所有用户都有手机号 | `user_credentials.identifier`（auth_type=phone） |
| `email_verified` | 验证状态跟凭证走 | `user_credentials.verified` |

**新增字段：**

| 字段 | 说明 |
|------|------|
| `registered_via` | 用户最初注册方式（如 `email`、`oauth_github`），注册时写入，永不变更。用于引导流程、安全审计、产品分析 |

### 2.2 `user_credentials` 表（新增 — 所有登录凭证）

```sql
CREATE TABLE IF NOT EXISTS user_credentials (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    document_id TEXT NOT NULL UNIQUE,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    auth_type TEXT NOT NULL,                -- 凭证类型（见下方枚举）
    identifier TEXT NOT NULL,               -- 登录标识符
    credential_data TEXT NOT NULL,          -- JSON: 凭证数据（密码hash、公钥、OAuth token等）
    verified INTEGER NOT NULL DEFAULT 0,    -- 是否已验证
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    UNIQUE(auth_type, identifier)           -- 同类型同标识符唯一
);

CREATE INDEX IF NOT EXISTS idx_user_credentials_user ON user_credentials(user_id);
CREATE INDEX IF NOT EXISTS idx_user_credentials_type_id ON user_credentials(auth_type, identifier);
CREATE INDEX IF NOT EXISTS idx_user_credentials_type ON user_credentials(auth_type);
```

**核心设计：`credential_data` 是 JSON 字段。** 每种 `auth_type` 定义自己的 JSON schema，表结构永远不需要改。

### 2.3 `auth_type` 枚举值及 `credential_data` schema

| auth_type | identifier | credential_data | 说明 |
|-----------|-----------|----------------|------|
| `email` | email 地址 | `{"password_hash": "$argon2id$..."}` | 邮箱密码登录 |
| `phone` | 手机号 | `{}` | 手机验证码登录 |
| `oauth_github` | `github:{provider_user_id}` | `{"email": "a@b.com", "display_name": "Alice"}` | GitHub OAuth |
| `oauth_google` | `google:{provider_user_id}` | `{"email": "a@b.com", "display_name": "Alice"}` | Google OAuth |
| `oauth_wechat` | `wechat:{openid}` | `{"union_id": "..."}` | 微信 OAuth |
| `passkey` | WebAuthn credential ID | `{"public_key": "base64...", "sign_count": 0, "transports": ["usb","ble"]}` | FIDO2/WebAuthn |
| `ldap` (预留) | DN 或用户名 | `{"dn": "cn=alice,ou=users,dc=corp"}` | 企业 LDAP |
| `saml` (预留) | NameID | `{"issuer": "https://idp.corp", "name_id": "alice"}` | 企业 SSO |

**新增登录方式**：只需定义一个新的 `auth_type` 字符串 + 对应的 `credential_data` JSON schema，表结构永远不改。

**设计原则：**
- 固定列只有 `auth_type`、`identifier`、`credential_data`、`mfa_data`
- 所有 auth_type 特有的数据全部放 JSON
- 新增登录方式 = 新增一行代码里的 enum variant + 新的 service 函数
- 不需要 ALTER TABLE

### 2.4 `oauth_accounts` 表（保留但精简）

`oauth_accounts` 表保留用于存储 OAuth token 等Provider特有的信息，与 `user_credentials` 为一对一关系：

```sql
-- 保留现有 oauth_accounts 表结构不变
-- 新增 user_credentials 后，oauth_accounts.user_id 与 credentials.user_id 对齐
-- oauthAccounts 仍负责存储 access_token / refresh_token / profile 等OAuth专属数据
```

### 2.5 其他表不变

- `refresh_tokens` — 不变
- `password_reset_tokens` — 不变
- `sms_codes` — 不变
- `email_verification_tokens` — 不变
- `oauth_states` — 不变

---

## 3. 数据模型

### 3.1 Rust Model

```rust
// src/models/user.rs — 精简后的 User
pub struct User {
    pub id: i64,
    pub document_id: String,
    pub tenant_id: Option<String>,
    pub username: String,
    pub display_name: Option<String>,
    pub avatar: Option<String>,
    pub bio: Option<String>,
    pub website: Option<String>,
    pub slug: Option<String>,
    pub locale: Option<String>,
    pub role: String,
    pub status: String,
    pub registered_via: String,      // 'email' | 'phone' | 'oauth_github' | ...
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

// src/models/user_credential.rs — 新增
pub struct UserCredential {
    pub id: i64,
    pub document_id: String,
    pub user_id: i64,
    pub auth_type: String,            // "email" | "phone" | "oauth_github" | ...
    pub identifier: String,           // email / phone / "github:12345"
    pub credential_data: String,      // JSON: { "password_hash": "..." } 或 {} 或 { "public_key": "..." }
    pub verified: i64,                 // 0 = 未验证, 1 = 已验证 (SQLite INTEGER)
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}
```

### 3.2 CreateUserCmd 变更

```rust
// 之前
pub struct CreateUserCmd {
    pub email: String,         // NOT NULL，被迫填哨兵值
    pub username: String,
    pub password_hash: String, // NOT NULL，被迫填哨兵值
}

// 之后
pub struct CreateUserCmd {
    pub username: String,
    pub registered_via: String,     // 'email' | 'phone' | 'oauth_github' | ...
}

// 凭证由各 service 独立通过 user_credential::create() 创建
// 不需要在 CreateUserCmd 中内嵌凭证信息
```

---

## 4. 注册流程

### 4.1 邮箱密码注册

```
POST /api/v1/auth/register
Body: { "username": "alice", "email": "alice@example.com", "password": "Secret123!" }

1. 检查 config.registration_email_enabled == true
2. 校验 email 格式、密码强度、username 唯一
3. 事务:
   a. INSERT users (username, role='reader')
   b. INSERT user_credentials (auth_type='email', identifier=email, credential_data=json(password_hash))
4. 如果 config.require_email_verification:
   - 创建 email_verification_token
   - 发送验证邮件
5. 返回 LoginResponse (如果不需要验证) 或 UserResponse (如果需要验证)
```

### 4.2 手机号注册（短信验证码）

```
Step 1: POST /api/v1/auth/sms/send
Body: { "phone": "13800001234", "purpose": "register" }

1. 检查 config.registration_sms_enabled == true
2. 限流检查
3. 生成验证码，存入 sms_codes 表
4. 发送短信

Step 2: POST /api/v1/auth/sms/verify
Body: { "phone": "13800001234", "code": "123456", "purpose": "register" }

1. 校验验证码
2. 事务:
   a. 查找 user_credentials WHERE auth_type='phone' AND identifier=phone
   b. 如果已存在 → 登录（签发 JWT）
   c. 如果不存在:
      - INSERT users (username='user_13800001234')
      - INSERT user_credentials (auth_type='phone', identifier=phone, verified=true)
      - 签发 JWT
3. 返回 LoginResponse
```

### 4.3 OAuth 注册

```
Step 1: GET /api/v1/auth/oauth/{provider} → 302 到 Provider

Step 2: GET /api/v1/auth/oauth/{provider}/callback?code=xxx&state=yyy

1. 交换 code → access_token
2. 获取 Provider 用户信息 (provider_user_id, email, display_name, avatar)
3. 查找 user_credentials WHERE auth_type='oauth_{provider}' AND identifier='{provider}:{provider_user_id}'
4. 如果已存在 → 找到 user_id → 签发 JWT
5. 如果不存在:
   a. 查找 user_credentials WHERE auth_type='email' AND identifier=email (如果 provider 返回了 email)
      - 如果找到 → 自动绑定（增加一条 credential）
   b. 否则:
      - 事务:
        - INSERT users (username=display_name 或 '{provider}_{id[:8]}')
        - INSERT user_credentials (auth_type='oauth_{provider}', identifier='github:12345', verified=true)
        - INSERT oauth_accounts (token 等信息)
        - 如果 provider 返回了 email:
          INSERT user_credentials (auth_type='email', identifier=email, credential_data='{}', verified=true)
      - 签发 JWT
6. 返回 LoginResponse (或 302 重定向到前端)
```

---

## 5. 登录流程

### 5.1 邮箱密码登录

```
POST /api/v1/auth/login
Body: { "email": "alice@example.com", "password": "Secret123!" }

1. 查找 user_credentials WHERE auth_type='email' AND identifier=email
2. 如果未找到 → Unauthorized
3. 验证 secret (Argon2id)
4. 检查 verified (如果 require_email_verification)
5. 通过 user_id 找到 user
6. 签发 JWT + refresh_token
7. 返回 LoginResponse
```

### 5.2 手机号登录

与 4.2 的 verify 流程相同 — 验证码验证成功即登录。

### 5.3 OAuth 登录

与 4.3 的 callback 流程相同 — 找到已绑定的 credential 即登录。

---

## 6. 凭证绑定与解绑

### 6.1 绑定新凭证（已登录用户）

所有绑定操作的通用模式：

```
POST /api/v1/auth/credentials/bind
Body: { "auth_type": "email", "identifier": "alice@gmail.com", "secret": "NewPass123!" }
Auth: Bearer token (必须已登录)

前置检查:
1. 用户已有凭证数量 >= 1
2. 该 auth_type + identifier 不存在（或属于当前用户）

结果: INSERT user_credentials 一行
```

具体端点：

| 操作 | 端点 | 实现 |
|------|------|------|
| 绑定邮箱+密码 | `POST /auth/credentials/bind-email` | auth_type=email, 需要 email 验证 |
| 绑定手机号 | `POST /auth/phone/bind` | auth_type=phone, 需要 SMS 验证 |
| 绑定 OAuth | `GET /auth/oauth/{provider}` (已登录) | auth_type=oauth_{provider}, OAuth 流程 |

### 6.2 解绑凭证

```
DELETE /api/v1/auth/credentials/{credential_id}
Auth: Bearer token (必须已登录)

前置检查:
1. 该 credential 属于当前用户
2. 该用户剩余 credential 数量 > 1 (防止锁死)
3. 如果是最后一个凭证 → 返回 400 "cannot_remove_last_credential"

结果: DELETE user_credentials 一行
```

### 6.3 设置密码（OAuth/手机号用户首次设密码）

```
POST /api/v1/auth/set-password
Body: { "email": "alice@gmail.com", "password": "NewPass123!" }
Auth: Bearer token (必须已登录)

1. 检查当前用户是否已有 email 凭证
   - 如果有 → 400 "password_already_set"
   - 如果没有 → 创建新凭证:
     INSERT user_credentials (auth_type='email', identifier=email, credential_data=json(password_hash))
2. 返回 200
```

---

## 7. 密码管理

### 7.1 修改密码（已有密码的用户）

```
POST /api/v1/auth/change-password
Body: { "old_password": "...", "new_password": "..." }
Auth: Bearer token

1. 找到当前用户的 email 凭证
2. 验证 old_password (Argon2id)
3. 更新 secret 为新 hash
```

### 7.2 忘记密码（未登录）

```
Step 1: POST /api/v1/auth/forgot-password
Body: { "email": "alice@example.com" }

1. 查找 user_credentials WHERE auth_type='email' AND identifier=email
2. 生成 password_reset_token
3. 发送重置邮件

Step 2: POST /api/v1/auth/reset-password
Body: { "token": "...", "new_password": "..." }

1. 验证 token
2. 更新对应 credential 的 secret
```

### 7.3 短信重置密码（无邮箱用户）

```
Step 1: POST /api/v1/auth/sms/send
Body: { "phone": "13800001234", "purpose": "reset_password" }

Step 2: POST /api/v1/auth/sms/reset-password
Body: { "phone": "13800001234", "code": "123456", "new_password": "..." }

1. 验证 SMS code
2. 找到用户，创建 email 凭证 (需要前端提供 email) 或直接更新 phone 凭证
```

---

## 8. 配置项

| 环境变量 | 类型 | 默认值 | 说明 |
|---------|------|--------|------|
| `REGISTRATION_EMAIL_ENABLED` | bool | `true` | 允许邮箱密码注册 |
| `REGISTRATION_SMS_ENABLED` | bool | `false` | 允许手机号注册 |
| `OAUTH_ENABLED` | bool | `false` | 启用 OAuth |
| `REQUIRE_EMAIL_VERIFICATION` | bool | `false` | 邮箱注册后是否强制验证 |
| `OAUTH_GITHUB_CLIENT_ID` | string | — | GitHub OAuth |
| `OAUTH_GITHUB_CLIENT_SECRET` | string | — | GitHub OAuth |
| `OAUTH_GOOGLE_CLIENT_ID` | string | — | Google OAuth |
| `OAUTH_GOOGLE_CLIENT_SECRET` | string | — | Google OAuth |
| `OAUTH_WECHAT_APP_ID` | string | — | 微信 OAuth |
| `OAUTH_WECHAT_APP_SECRET` | string | — | 微信 OAuth |
| `SMS_CODE_LENGTH` | u32 | `6` | 短信验证码位数 |
| `SMS_CODE_EXPIRES_IN` | u64 | `300` | 验证码过期秒数 |
| `SMS_RATE_LIMIT_SECS` | u64 | `60` | 发送间隔秒数 |

---

## 9. API 端点汇总

### 9.1 公开端点（无需认证）

| Method | Path | 说明 |
|--------|------|------|
| POST | `/auth/register` | 邮箱密码注册 |
| POST | `/auth/login` | 邮箱密码登录 |
| POST | `/auth/sms/send` | 发送短信验证码 |
| POST | `/auth/sms/verify` | 短信验证码登录/注册 |
| POST | `/auth/refresh` | 刷新 access token |
| POST | `/auth/forgot-password` | 忘记密码（发邮件） |
| POST | `/auth/reset-password` | 重置密码 |
| POST | `/auth/verify-email` | 验证邮箱 |
| POST | `/auth/resend-verification` | 重发验证邮件 |
| GET | `/auth/config` | 获取登录配置（哪些方式可用） |
| GET | `/auth/oauth/{provider}` | 发起 OAuth |
| GET | `/auth/oauth/{provider}/callback` | OAuth 回调 |

### 9.2 认证端点（需要 Bearer token）

| Method | Path | 说明 |
|--------|------|------|
| POST | `/auth/logout` | 登出（吊销 refresh token） |
| POST | `/auth/change-password` | 修改密码 |
| POST | `/auth/set-password` | 首次设置密码 |
| POST | `/auth/phone/bind` | 绑定手机号 |
| POST | `/auth/credentials/bind-email` | 绑定邮箱密码 |
| DELETE | `/auth/credentials/{id}` | 解绑指定凭证 |
| GET | `/auth/credentials` | 列出当前用户所有凭证 |
| GET | `/auth/oauth/bindings` | 列出 OAuth 绑定 |
| DELETE | `/auth/oauth/{provider}/unbind` | 解绑 OAuth |

---

## 10. 安全策略

### 10.1 凭证数量保护

```
删除凭证前检查:
  SELECT COUNT(*) FROM user_credentials WHERE user_id = ?
  如果 count <= 1 → 拒绝删除，返回 400 "cannot_remove_last_credential"
```

### 10.2 OAuth 自动绑定策略

当 OAuth provider 返回的 email 与已有用户匹配时：

| 策略 | 行为 | 适用场景 |
|------|------|---------|
| `auto` (当前) | 自动静默绑定 | 低安全要求 |
| `confirm` (推荐) | 返回待确认状态，前端让用户确认后绑定 | 默认 |
| `disabled` | 不自动绑定，创建新用户 | 高安全要求 |

通过 `OAUTH_EMAIL_BIND_POLICY` 环境变量配置。

### 10.3 密码强度

沿用当前规则：
- 最少 8 字符
- 必须包含字母和数字
- Argon2id 哈希

---

## 11. 旧数据迁移

### 11.1 迁移脚本

```sql
-- Step 1: 创建 user_credentials 表 (见 2.2)

-- Step 2: 迁移邮箱密码凭证
INSERT INTO user_credentials (document_id, user_id, auth_type, identifier, credential_data, verified, created_at, updated_at)
SELECT
    lower(hex(randomblob(16))),
    id,
    'email',
    email,
    json_quote(password_hash),
    email_verified,
    created_at,
    updated_at
FROM users
WHERE email NOT LIKE '!sms:%' AND email != '';

-- Step 3: 迁移手机号凭证（从 !sms: 哨兵值恢复）
INSERT INTO user_credentials (document_id, user_id, auth_type, identifier, credential_data, verified, created_at, updated_at)
SELECT
    lower(hex(randomblob(16))),
    id,
    'phone',
    substr(email, 6),   -- 去掉 '!sms:' 前缀，得到真实手机号
    '{}',                -- 无密码
    1,
    created_at,
    updated_at
FROM users
WHERE email LIKE '!sms:%';

-- Step 4: 迁移 OAuth 凭证
INSERT INTO user_credentials (document_id, user_id, auth_type, identifier, credential_data, verified, created_at, updated_at)
SELECT
    lower(hex(randomblob(16))),
    oa.user_id,
    'oauth_' || oa.provider,
    oa.provider || ':' || oa.provider_user_id,
    json_object('email', oa.email, 'display_name', oa.display_name),
    1,
    oa.created_at,
    oa.updated_at
FROM oauth_accounts oa;

-- Step 5: 从 users 表移除旧字段
-- (SQLite 不支持 DROP COLUMN，需要重建表)
-- 在新 schema 中直接不包含 email, password_hash, phone, email_verified
```

### 11.2 迁移注意事项

- 先备份 `storage/db/raisfast.db`
- 迁移后 `users` 表不再有 `email`、`password_hash`、`phone`、`email_verified`
- `oauth_accounts` 表保留（存储 OAuth token）
- `sms_codes`、`email_verification_tokens`、`password_reset_tokens` 表不变

---

## 12. 文件变更清单

| 文件 | 变更 |
|------|------|
| `migrations/sqlite/schema.sqlite.sql` | users 表精简 + 新增 user_credentials 表 |
| `migrations/postgres/schema.postgres.sql` | 同上 |
| `migrations/mysql/schema.mysql.sql` | 同上 |
| `src/models/user.rs` | 移除 email/password_hash/phone/email_verified 字段 |
| `src/models/user_credential.rs` | **新增** — UserCredential model + CRUD |
| `src/commands/user.rs` | CreateUserCmd 改为 username + registered_via |
| `src/repositories/sqlx_user.rs` | 简化，移除 find_by_email/find_by_phone/update_password |
| `src/services/auth.rs` | 重写注册/登录，通过 credential 查找 |
| `src/services/sms.rs` | 重写，创建 phone credential 而非哨兵 email |
| `src/services/oauth.rs` | 重写，创建 oauth_* credential，去掉哨兵 |
| `src/services/password_reset.rs` | 适配，操作 credential 而非 user.password_hash |
| `src/services/email_verification.rs` | 适配，更新 credential.verified |
| `src/handlers/auth.rs` | 调整 DTO + 调用新 service 接口 |
| `src/dto/user.rs` | RegisterRequest/LoginRequest 调整 |
| `src/dto/user.rs` | UserResponse 移除 email/phone（改为从 credentials 获取） |

---

## 13. 实施步骤

```
Phase 1: Schema + Model (0.5 天)
  1. 更新 schema.sql 三份
  2. 创建 user_credential model + CRUD
  3. 更新 User model
  4. 编译通过

Phase 2: Service 重写 (1 天)
  5. 重写 auth.rs 注册/登录
  6. 重写 sms.rs
  7. 重写 oauth.rs
  8. 重写 password_reset.rs
  9. 重写 email_verification.rs

Phase 3: Handler + DTO (0.5 天)
  10. 更新 auth handler
  11. 更新 user DTO
  12. 新增 credentials 端点

Phase 4: 测试 (0.5 天)
  13. 更新集成测试
  14. 新增 credential 绑定/解绑测试
  15. 迁移脚本测试

Phase 5: 迁移 (0.5 天)
  16. 编写数据迁移脚本
  17. 全量回归测试
```

---

## 14. MFA（多因素认证）

### 14.1 设计原则

MFA 是**用户级别**的功能，独立于登录方式。不管用户用邮箱、手机、还是 OAuth 登录，MFA 验证都是同一套。

```
登录流程:
  验证主凭证（密码/SMS/OAuth）→ 检查 MFA → 签发 JWT
                                  ↓
                          user_mfa 表有记录?
                          ├── 否 → 直接签发 JWT
                          └── 是 → 返回 mfa_required，需二次验证
```

MFA 与凭证分离的好处：
- 用户换登录方式不影响 MFA 设置
- 新增 MFA 方式不改 `user_credentials` 表
- 可以同时启用多种 MFA（TOTP 作为主方式，恢复码作为备用）

### 14.2 `user_mfa` 表（新增）

```sql
CREATE TABLE IF NOT EXISTS user_mfa (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    document_id TEXT NOT NULL UNIQUE,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    method TEXT NOT NULL,                   -- 'totp' | 'webauthn' | 'sms' | 'email_otp'
    mfa_data TEXT NOT NULL,                 -- JSON: 方法特定数据（见下方 schema）
    verified INTEGER NOT NULL DEFAULT 0,    -- 设置流程中是否已确认
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    UNIQUE(user_id, method)                 -- 每种方式每用户一行
);

CREATE INDEX IF NOT EXISTS idx_user_mfa_user ON user_mfa(user_id);
```

### 14.3 `mfa_data` JSON schema

每种 MFA 方法定义自己的数据结构，`mfa_data` 字段永远不需要 ALTER TABLE：

| method | mfa_data | 说明 |
|--------|----------|------|
| `totp` | `{"secret": "JBSWY3DPEHPK3PXP", "algorithm": "SHA1", "digits": 6, "period": 30}` | RFC 6238 TOTP，Microsoft/Google Authenticator |
| `webauthn` | `{"credential_id": "base64...", "public_key": "base64...", "sign_count": 0, "transports": ["usb","ble"]}` | FIDO2 / Passkey，未来最高安全等级 |
| `sms` | `{}` | 复用 `sms_codes` 表，发验证码到绑定手机 |
| `email_otp` | `{}` | 复用 `email_verification_tokens` 表，发验证码到绑定邮箱 |

### 14.4 `user_mfa_recovery_codes` 表（新增）

```sql
CREATE TABLE IF NOT EXISTS user_mfa_recovery_codes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    code_hash TEXT NOT NULL,                -- bcrypt hash
    used_at TEXT,                           -- NULL = 未使用
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_mfa_recovery_user ON user_mfa_recovery_codes(user_id);
```

### 14.5 MFA 流程

#### 14.5.1 登录时触发 MFA

```json
// Step 1: 正常登录请求（任何登录方式）
POST /api/v1/auth/login
Body: { "email": "alice@example.com", "password": "Secret123!" }

// 如果该用户启用了 MFA，返回:
{
  "code": 0,
  "message": "mfa_required",
  "data": {
    "status": "mfa_required",
    "mfa_token": "eyJ...short-lived-token...",
    "methods": ["totp"],
    "hint": "a***@gmail.com"
  }
}

// mfa_token 有效期 5 分钟，只能用于完成 MFA 验证，不能当作 access_token

// Step 2: MFA 验证
POST /api/v1/auth/mfa/verify
Body: { "mfa_token": "eyJ...", "method": "totp", "code": "123456" }

// 验证通过后返回正常的 LoginResponse:
{
  "code": 0,
  "message": "success",
  "data": {
    "access_token": "eyJ...",
    "refresh_token": "...",
    "expires_in": 3600,
    "user": { ... }
  }
}
```

#### 14.5.2 所有登录方式统一触发

| 登录方式 | 主凭证验证后 | MFA 检查 |
|---------|-----------|---------|
| 邮箱密码 | 密码正确 → | `SELECT * FROM user_mfa WHERE user_id = ?` |
| 手机短信 | 验证码正确 → | 同上 |
| OAuth | provider 验证通过 → | 同上 |
| Passkey | 签名验证通过 → | 同上 |

### 14.6 MFA 设置（绑定）

#### 14.6.1 启用 TOTP

```
Step 1: POST /api/v1/auth/mfa/setup
Body: { "method": "totp" }
Auth: Bearer token

→ 生成 Base32 secret
→ 返回:
{
  "secret": "JBSWY3DPEHPK3PXP",
  "qr_url": "otpauth://totp/raisfast:alice?secret=JBSWY3DPEHPK3PXP&issuer=raisfast",
  "backup_codes": ["abc12345", "def67890", ...]
}

Step 2: POST /api/v1/auth/mfa/confirm
Body: { "method": "totp", "code": "123456" }

→ 验证 TOTP code 正确
→ INSERT user_mfa (user_id, method='totp', mfa_data='{"secret":"JBSWY3DPEHPK3PXP",...}', verified=1)
→ INSERT user_mfa_recovery_codes × 10 条
```

#### 14.6.2 启用 WebAuthn / Passkey（未来）

```
Step 1: POST /api/v1/auth/mfa/setup
Body: { "method": "webauthn" }

→ 生成 challenge，返回 WebAuthn registration options
→ 前端调用 navigator.credentials.create()

Step 2: POST /api/v1/auth/mfa/confirm
Body: { "method": "webauthn", "credential": { "id": "...", "publicKey": "...", ... } }

→ 验证签名
→ INSERT user_mfa (user_id, method='webauthn', mfa_data='{...}')
```

#### 14.6.3 启用短信/邮箱 OTP

```
POST /api/v1/auth/mfa/setup
Body: { "method": "sms" }

→ 检查用户是否已绑定手机号（有 phone credential）
→ INSERT user_mfa (user_id, method='sms', mfa_data='{}')
```

### 14.7 MFA 解除

```
DELETE /api/v1/auth/mfa
Body: { "code": "123456" }  // 需要当前 MFA 验证码确认
Auth: Bearer token

→ 验证 code 正确后:
  DELETE FROM user_mfa WHERE user_id = ?
  DELETE FROM user_mfa_recovery_codes WHERE user_id = ?
```

### 14.8 恢复码使用

```
POST /api/v1/auth/mfa/verify
Body: { "mfa_token": "eyJ...", "method": "recovery_code", "code": "abc12345" }

→ 查找 user_mfa_recovery_codes WHERE user_id=? AND code_hash 匹配 AND used_at IS NULL
→ 标记 used_at = now()
→ 签发 JWT
→ 如果剩余未使用恢复码 < 3，提示用户重新生成
```

### 14.9 MFA API 端点

| Method | Path | Auth | 说明 |
|--------|------|------|------|
| POST | `/auth/mfa/verify` | mfa_token | 完成 MFA 二次验证 |
| POST | `/auth/mfa/setup` | Bearer | 发起 MFA 设置（返回 secret/QR/challenge） |
| POST | `/auth/mfa/confirm` | Bearer | 确认 MFA 设置（验证码/签名确认） |
| DELETE | `/auth/mfa` | Bearer | 关闭 MFA（需验证码） |
| GET | `/auth/mfa/status` | Bearer | 查询当前 MFA 状态 |
| POST | `/auth/mfa/recovery-codes/regenerate` | Bearer | 重新生成恢复码 |

### 14.10 安全策略

| 策略 | 说明 |
|------|------|
| mfa_token 有效期 5 分钟 | 一次性使用，验证后作废 |
| TOTP 容差 ±1 个时间窗口（30s×3） | 防止时钟偏移导致失败 |
| 恢复码使用后立即作废 | 每个码只能用一次 |
| 关闭 MFA 需要验证码 | 防止攻击者直接关闭 |
| 恢复码以 hash 存储 | 数据库泄露后无法直接使用 |
| 可配置强制 MFA | `REQUIRE_MFA=true` 时管理员/作者角色必须启用 MFA |
| TOTP secret 加密存储 | `mfa_data.secret` 用 AES-256-GCM 加密，key 从 `MFA_ENCRYPTION_KEY` 读取 |

### 14.11 未来扩展

新增 MFA 方式只需：

1. 定义新的 `method` 字符串（如 `"webauthn"`）
2. 定义对应的 `mfa_data` JSON schema
3. 实现该方法的验证逻辑 service

不需要 ALTER TABLE，不需要改 `user_credentials`，不需要改登录流程。

---

## 15. 实名认证（未来计划，暂不实施）

### 15.1 设计思路

实名认证与登录凭证是**两个独立维度**：
- 凭证 = 你怎么证明你是你（密码、OAuth、MFA）
- 实名 = 你在现实世界是谁（身份证、护照、营业执照）

实名认证是**用户级别的身份属性**，独立于任何登录方式。

### 15.2 `user_identity_verifications` 表（未来创建）

```sql
CREATE TABLE IF NOT EXISTS user_identity_verifications (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    document_id TEXT NOT NULL UNIQUE,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    verification_type TEXT NOT NULL,        -- 'id_card' | 'passport' | 'business_license' | 'phone_realname'
    status TEXT NOT NULL DEFAULT 'pending', -- 'pending' | 'verified' | 'rejected'
    id_number TEXT,                         -- AES-256-GCM 加密存储的证件号
    real_name TEXT,                         -- AES-256-GCM 加密存储的真实姓名
    verification_data TEXT,                 -- JSON: 第三方认证返回、认证凭证、OCR 结果等
    verified_at TEXT,
    rejected_reason TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    UNIQUE(user_id, verification_type)
);

CREATE INDEX IF NOT EXISTS idx_identity_verifications_user ON user_identity_verifications(user_id);
CREATE INDEX IF NOT EXISTS idx_identity_verifications_status ON user_identity_verifications(status);
```

### 15.3 `verification_type` 及 `verification_data` schema

| verification_type | verification_data | 说明 |
|-------------------|-------------------|------|
| `id_card` | `{"front_image":"...","back_image":"...","ocr_result":{...},"third_party_ref":"..."}` | 中国大陆身份证 |
| `passport` | `{"passport_image":"...","ocr_result":{...}}` | 护照 |
| `business_license` | `{"license_image":"...","company_name":"...","unified_code":"..."}` | 企业营业执照 |
| `phone_realname` | `{"phone":"138...","carrier":"cmcc"}` | 运营商三要素实名 |
| `bank_card` | `{"bank":"ICBC","card_last4":"1234"}` | 银行卡四要素实名 |

### 15.4 实名认证流程（未来实现）

```
Step 1: POST /api/v1/auth/identity/submit
Body: { "type": "id_card", "real_name": "张三", "id_number": "110...", "front_image": "base64..." }
Auth: Bearer token

→ 创建验证记录，status='pending'
→ 调用第三方实名认证 API（阿里云 / 腾讯云）
→ 更新 status='verified' 或 'rejected'

Step 2: GET /api/v1/auth/identity/status
Auth: Bearer token

→ 返回当前实名状态
```

### 15.5 安全要求

| 要求 | 说明 |
|------|------|
| 证件号加密存储 | `id_number` 和 `real_name` 用 AES-256-GCM 加密，key 从 `IDENTITY_ENCRYPTION_KEY` 读取 |
| 查询脱敏 | API 返回时脱敏：`id_number` → `"110***********1234"`，`real_name` → `"张*"` |
| 不可逆 | 不提供明文查询接口，只返回是否已认证 + 脱敏信息 |
| 审计日志 | 所有实名认证操作记录到 audit log |

### 15.6 与权限系统联动（未来）

```
配置: REQUIRE_IDENTITY_VERIFICATION=true

未实名用户:
  - 可以登录
  - 可以浏览内容
  - 不能发布内容（POST /api/v1/posts → 403 "identity_verification_required"）
  - 不能提现 / 操作敏感功能

已实名用户:
  - 解锁全部功能
```
