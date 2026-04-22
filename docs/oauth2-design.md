# OAuth2 社交登录设计方案

## 概述

支持 GitHub、Google、微信等第三方 OAuth2 登录，采用标准 Authorization Code + PKCE 流程。
首次 OAuth 登录自动创建本地用户，已有账号可绑定/解绑 OAuth Provider。

## 流程

```
用户点击 "GitHub 登录"
  → GET /api/v1/auth/oauth/{provider}
      → 302 重定向到 OAuth Provider 授权页

  → 用户在 Provider 授权
  → Provider 回调
  → GET /api/v1/auth/oauth/{provider}/callback?code=xxx&state=xxx
      → 后端用 code + code_verifier 换 access_token
      → 用 access_token 调用 Provider 用户信息 API
      → 查找或创建本地用户
      → 签发 JWT access_token + refresh_token
      → 302 重定向前端回调页，URL 拼接 token 参数

  → 前端从 URL 提取 token，存入 localStorage / cookie
```

## API 端点

### 1. 发起 OAuth 登录

```
GET /api/v1/auth/oauth/{provider}
```

- `{provider}`: `github` | `google` | `wechat`
- 生成 `state`（CSRF token）和 `code_verifier`（PKCE），存入 DB 或 Redis
- 302 重定向到 Provider 授权 URL
- 可选查询参数 `?bind=1`，表示已登录用户绑定第三方账号

### 2. OAuth 回调

```
GET /api/v1/auth/oauth/{provider}/callback?code=xxx&state=xxx
```

- 校验 `state` 防止 CSRF
- 用 `code` + `code_verifier` 换 Provider 的 access_token
- 获取 Provider 用户信息（ID、邮箱、昵称、头像）
- 查找 `oauth_accounts` 表：
  - 已绑定 → 直接签发 token
  - 未绑定 + `bind=1` → 绑定到当前登录用户
  - 未绑定 + 邮箱匹配已有用户 → 自动绑定（可选，需评估安全风险）
  - 未绑定 + 新用户 → 自动注册
- 302 重定向到 `OAUTH_REDIRECT_URL?access_token=...&refresh_token=...&expires_in=...`

### 3. 绑定 OAuth 账号（已登录用户）

```
POST /api/v1/auth/oauth/bind
Authorization: Bearer <token>

{ "provider": "github" }
```

- 返回绑定用的授权 URL
- 前端跳转该 URL，回调时自动绑定

### 4. 解绑 OAuth 账号

```
DELETE /api/v1/auth/oauth/{provider}
Authorization: Bearer <token>
```

- 解绑指定 Provider
- 若用户无密码且仅剩一个绑定，拒绝解绑（防止无法登录）

### 5. 查询已绑定的 Provider 列表

```
GET /api/v1/auth/oauth/providers
Authorization: Bearer <token>
```

- 返回当前用户已绑定的 Provider 列表
- 前端用于展示绑定状态和解绑操作

## 数据库

### 新表 `oauth_accounts`

```sql
CREATE TABLE oauth_accounts (
    id TEXT PRIMARY KEY,              -- UUID v7
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider TEXT NOT NULL,           -- 'github' | 'google' | 'wechat'
    provider_user_id TEXT NOT NULL,   -- Provider 侧的用户 ID
    email TEXT,                       -- Provider 返回的邮箱
    display_name TEXT,                -- Provider 返回的昵称
    avatar_url TEXT,                  -- Provider 返回的头像 URL
    access_token TEXT,                -- Provider 的 access_token（可选，加密存储）
    refresh_token TEXT,               -- Provider 的 refresh_token（可选，加密存储）
    token_expires_at TEXT,            -- Provider token 过期时间
    profile TEXT,                     -- Provider 返回的原始 profile JSON
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(provider, provider_user_id)
);

CREATE INDEX idx_oauth_accounts_user ON oauth_accounts(user_id);
CREATE INDEX idx_oauth_accounts_provider ON oauth_accounts(provider, provider_user_id);
```

### 新表 `oauth_states`（短期 PKCE state 存储）

```sql
CREATE TABLE oauth_states (
    id TEXT PRIMARY KEY,              -- 即 state 值本身（随机字符串）
    provider TEXT NOT NULL,
    code_verifier TEXT NOT NULL,      -- PKCE code_verifier
    user_id TEXT,                     -- 绑定场景下传入已登录用户 ID
    redirect_url TEXT,                -- 可选自定义回调地址
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    expires_at TEXT NOT NULL          -- 默认 10 分钟后过期
);

-- 定期清理过期记录（可由 Worker cron 处理）
CREATE INDEX idx_oauth_states_expires ON oauth_states(expires_at);
```

### `users` 表改动

```sql
-- password_hash 改为可空（纯 OAuth 用户无密码）
-- 新增迁移：
ALTER TABLE users RENAME COLUMN password_hash TO password_hash_old;
ALTER TABLE users ADD COLUMN password_hash TEXT;
UPDATE users SET password_hash = password_hash_old;
ALTER TABLE users DROP COLUMN password_hash_old;

-- 新增邮箱验证标记（OAuth 登录的邮箱默认已验证）
ALTER TABLE users ADD COLUMN email_verified INTEGER NOT NULL DEFAULT 0;
```

> 注意：实际迁移 SQL 需根据 SQLite 的 ALTER TABLE 限制分步执行。

## Provider 配置

### 环境变量

```env
# 通用
OAUTH_ENABLED=true
OAUTH_REDIRECT_URL=http://localhost:3000/auth/callback

# GitHub
OAUTH_GITHUB_CLIENT_ID=
OAUTH_GITHUB_CLIENT_SECRET=

# Google
OAUTH_GOOGLE_CLIENT_ID=
OAUTH_GOOGLE_CLIENT_SECRET=

# 微信（注意：微信使用独立协议，非标准 OAuth2）
OAUTH_WECHAT_APP_ID=
OAUTH_WECHAT_APP_SECRET=
```

### Provider 定义

| Provider | Authorization URL | Token URL | User Info API | Scope |
|----------|------------------|-----------|---------------|-------|
| GitHub | `https://github.com/login/oauth/authorize` | `https://github.com/login/oauth/access_token` | `GET https://api.github.com/user` + `GET /user/emails` | `user:email` |
| Google | `https://accounts.google.com/o/oauth2/v2/auth` | `https://oauth2.googleapis.com/token` | `GET https://www.googleapis.com/oauth2/v2/userinfo` | `openid email profile` |
| 微信 | `https://open.weixin.qq.com/connect/qrconnect` | `https://api.weixin.qq.com/sns/oauth2/access_token` | `GET https://api.weixin.qq.com/sns/userinfo` | `snsapi_login` |

### AppConfig 新增字段

```rust
pub struct OAuthConfig {
    pub enabled: bool,
    pub redirect_url: String,
    pub github: Option<GitHubConfig>,
    pub google: Option<GoogleConfig>,
    pub wechat: Option<WechatConfig>,
}

pub struct GitHubConfig {
    pub client_id: String,
    pub client_secret: String,
}

pub struct GoogleConfig {
    pub client_id: String,
    pub client_secret: String,
}

pub struct WechatConfig {
    pub app_id: String,
    pub app_secret: String,
}
```

### Provider 注册表

```rust
pub struct OAuthProviderRegistry {
    providers: HashMap<String, Box<dyn OAuthProvider>>,
}

pub trait OAuthProvider: Send + Sync {
    /// 返回 Provider 标识（如 "github"）
    fn name(&self) -> &str;

    /// 构建授权 URL（包含 state、code_challenge、scope）
    fn authorize_url(&self, state: &str, code_challenge: &str) -> String;

    /// 用 code + code_verifier 换 access_token
    async fn exchange_code(&self, code: &str, code_verifier: &str) -> Result<OAuthTokenResponse>;

    /// 用 access_token 获取用户信息
    async fn fetch_user_info(&self, access_token: &str) -> Result<OAuthUserInfo>;
}

pub struct OAuthUserInfo {
    pub provider_user_id: String,
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub raw_profile: serde_json::Value,
}
```

## 自动注册策略

OAuth 首次登录且未绑定已有账号时，自动创建本地用户：

1. **username 生成**：
   - 优先使用 Provider 返回的 `login`（GitHub）或 `name`
   - 若已存在，追加 Provider 前缀：`github_octocat`
   - 若仍冲突，追加随机后缀：`github_octocat_3f8a`

2. **email**：
   - 使用 Provider 返回的邮箱
   - GitHub 需额外调用 `/user/emails` API 获取主邮箱
   - Google 直接返回
   - 标记 `email_verified = true`

3. **password**：
   - `password_hash = NULL`（纯 OAuth 用户无密码）
   - 用户可后续设置密码（成为"混合登录"用户）

4. **role**：
   - 默认 `reader`，与普通注册一致

5. **avatar**：
   - 使用 Provider 返回的头像 URL

## 安全设计

### PKCE（Proof Key for Code Exchange）

- 每次授权请求生成随机 `code_verifier`（43-128 字符）
- `code_challenge = BASE64URL(SHA256(code_verifier))`
- `code_challenge_method = S256`
- 回调时发送 `code_verifier` 完成 PKCE 验证
- 防止 authorization code 被截获后滥用

### State 防护

- 每次请求生成随机 `state`，存入 `oauth_states` 表
- 回调时严格校验 `state` 参数
- `state` 10 分钟后自动过期
- 用后即删（一次性）

### Token 存储

- Provider 返回的 access_token / refresh_token 加密存储（AES-256-GCM）
- 加密密钥通过环境变量 `OAUTH_TOKEN_ENCRYPTION_KEY` 配置
- 若不需要 Provider 级别 API 代理功能，可选择不存储 Provider token

### 其他

- OAuth 登录同样受 rate_limit 保护
- 审计日志记录所有 OAuth 绑定/解绑操作
- 绑定操作需要二次确认（已登录 + 明确请求）

## 代码文件规划

| 文件 | 职责 |
|------|------|
| `src/config/oauth.rs` | `OAuthConfig`、Provider 子配置、环境变量加载 |
| `src/handlers/oauth.rs` | 路由处理器：redirect、callback、bind、unbind、list providers |
| `src/services/oauth.rs` | 业务逻辑：code exchange、find-or-create、绑定/解绑、JWT 签发 |
| `src/models/oauth.rs` | `oauth_accounts` + `oauth_states` 表 CRUD |
| `src/oauth/mod.rs` | `OAuthProviderRegistry`、trait 定义 |
| `src/oauth/github.rs` | GitHub Provider 实现 |
| `src/oauth/google.rs` | Google Provider 实现 |
| `src/oauth/wechat.rs` | 微信 Provider 实现（注意微信的非标准协议差异） |
| `migrations/019_oauth_accounts.sql` | `oauth_accounts` 建表 |
| `migrations/020_oauth_states.sql` | `oauth_states` 建表 |
| `migrations/021_users_oauth_compat.sql` | `users` 表 password_hash 可空 + email_verified 字段 |

## 实施优先级

### Phase 1：GitHub OAuth（推荐先做）

- 最简单，API 文档清晰
- 开发者友好，适合内部系统
- 只需 `reqwest`（已有依赖）做 HTTP 调用
- 无额外 crate 依赖

### Phase 2：Google OAuth

- 企业用户需求
- 使用 OpenID Connect（OAuth2 上层协议）
- 需处理 ID Token（JWT 格式）验证

### Phase 3：微信登录

- 国内用户刚需
- 微信协议非标准 OAuth2（无 PKCE、自定义错误码、unionid 机制）
- 需单独适配

### Phase 4：账号管理增强

- 邮箱验证流程（非 OAuth 注册用户）
- 密码重置（forgot password）
- 多 OAuth 账号合并
- Provider token 刷新机制

## 前端对接

### 登录按钮

前端在登录页渲染 OAuth 按钮：

```tsx
<a href={`${API_BASE}/auth/oauth/github`}>
  GitHub 登录
</a>
```

### 回调处理

```tsx
// /auth/callback 页面
useEffect(() => {
  const params = new URLSearchParams(window.location.search);
  const accessToken = params.get('access_token');
  const refreshToken = params.get('refresh_token');
  const expiresIn = params.get('expires_in');

  if (accessToken) {
    localStorage.setItem('access_token', accessToken);
    localStorage.setItem('refresh_token', refreshToken);
    navigate('/');
  }
}, []);
```

### 账号设置页

- 显示已绑定的 Provider 列表
- 提供绑定/解绑按钮
- 纯 OAuth 用户引导设置密码

## 依赖

无需新增 crate，使用现有依赖：

- `reqwest` — HTTP 调用（code exchange、user info）
- `serde_json` — JSON 解析
- `jsonwebtoken` — JWT 签发（复用现有）
- `sqlx` — 数据库操作
- `sha2` — PKCE code_challenge 生成（检查是否已引入）
- `base64` — BASE64URL 编码（检查是否已引入）

> 若 `sha2` 和 `base64` 尚未引入，需在 `Cargo.toml` 添加。
