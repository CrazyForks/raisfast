# OAuth2 用户注册/登录流程图

## 整体时序图

```
┌────────┐     ┌────────────┐     ┌──────────────┐     ┌──────────────────┐
│  前端   │     │ raisfast  │     │ GitHub OAuth │     │   SQLite DB      │
│ 浏览器  │     │  后端 API  │     │   Provider   │     │                  │
└───┬────┘     └─────┬──────┘     └──────┬───────┘     └────────┬─────────┘
    │                │                    │                      │
    │  1. GET /api/v1/auth/oauth/github  │                      │
    │───────────────>│                    │                      │
    │                │                    │                      │
    │                │  2. 检查 OAuth 是否启用                     │
    │                │  3. 检查 GitHub Provider 是否已配置          │
    │                │                    │                      │
    │                │  4. 生成 state (32字节随机hex)              │
    │                │  5. 生成 code_verifier (43字符随机串)       │
    │                │  6. 计算 code_challenge = BASE64URL(SHA256(code_verifier))
    │                │                    │                      │
    │                │  7. INSERT INTO oauth_states               │
    │                │    (state, provider, code_verifier,        │
    │                │     expires_at = now + 10min)              │
    │                │───────────────────────────────────────────>│
    │                │                    │                      │
    │                │  8. 构建 GitHub 授权 URL                    │
    │                │    https://github.com/login/oauth/authorize
    │                │    ?client_id=xxx                             │
    │                │    &state=xxx                                 │
    │                │    &code_challenge=xxx                        │
    │                │    &code_challenge_method=S256                │
    │                │    &scope=user:email                          │
    │                │                    │                      │
    │  9. 302 重定向到 GitHub 授权页    │                      │
    │<───────────────│                    │                      │
    │                │                    │                      │
    │  10. 用户在 GitHub 页面登录并授权  │                      │
    │────────────────────────────────────>│                      │
    │                │                    │                      │
    │  11. GitHub 回调带 code + state    │                      │
    │<────────────────────────────────────│                      │
    │                │                    │                      │
    │  12. GET /api/v1/auth/oauth/github/callback?code=xxx&state=xxx
    │───────────────>│                    │                      │
    │                │                    │                      │
    │                │  ┌─────────────────────────────────────┐  │
    │                │  │       回调处理核心流程                │  │
    │                │  │  (见下方详细流程图)                   │  │
    │                │  └─────────────────────────────────────┘  │
    │                │                    │                      │
    │  13. 302 重定向到前端回调页        │                      │
    │      ?access_token=xxx            │                      │
    │      &refresh_token=xxx           │                      │
    │      &expires_in=900              │                      │
    │<───────────────│                    │                      │
    │                │                    │                      │
    │  14. 前端提取 URL 参数，存入 localStorage，跳转首页       │
    │                │                    │                      │
```

## 回调处理核心流程

```
                        ┌─────────────────────┐
                        │  收到 callback 请求  │
                        │  provider + code    │
                        │  + state            │
                        └──────────┬──────────┘
                                   │
                                   ▼
                 ┌─────────────────────────────────┐
                 │  1. 检查 OAuth 是否启用           │
                 │     config.oauth.enabled == true │
                 └──────────────┬──────────────────┘
                                │
                          No ───┴─── Yes
                          │          │
                          ▼          ▼
                   ┌──────────┐  ┌──────────────────────────┐
                   │ 返回 400 │  │ 2. 从 DB 查找并消费 state  │
                   │ OAuth    │  │    SELECT * FROM           │
                   │ 未启用   │  │    oauth_states            │
                   └──────────┘  │    WHERE id = ?            │
                                 │    AND expires_at > now()  │
                                 │    然后 DELETE (一次性使用)  │
                                 └────────────┬───────────────┘
                                              │
                                    ┌─────────┴─────────┐
                                    │                   │
                              未找到/已过期          找到 state
                                    │                   │
                                    ▼                   ▼
                             ┌──────────┐  ┌──────────────────────┐
                             │ 返回 400 │  │ 3. 校验 provider 匹配  │
                             │ 无效或   │  │    state.provider ==   │
                             │ 过期的   │  │    请求的 provider     │
                             │ state    │  └──────────┬───────────┘
                             └──────────┘             │
                                                不匹配 │ 匹配
                                                      │ │
                                                      ▼ ▼
                                               ┌────────┐  ┌──────────────────────────┐
                                               │ 400    │  │ 4. 用 code 换 access_token │
                                               │        │  │    POST github.com/       │
                                               └────────┘  │    login/oauth/           │
                                                           │    access_token          │
                                                           │    ┌──────────────────┐  │
                                                           │    │ client_id       │  │
                                                           │    │ client_secret   │  │
                                                           │    │ code            │  │
                                                           │    │ code_verifier   │  │
                                                           │    └──────────────────┘  │
                                                           └──────────┬───────────────┘
                                                                      │
                                                                      ▼
                                                           ┌──────────────────────┐
                                                           │ 5. 获取 GitHub 用户信息│
                                                           │    GET api.github.com/ │
                                                           │    user               │
                                                           │    Header: Bearer token│
                                                           └──────────┬───────────┘
                                                                      │
                                                                      ▼
                                                           ┌──────────────────────┐
                                                           │ 6. 检查邮箱是否返回   │
                                                           │    profile.email ?    │
                                                           └──────────┬───────────┘
                                                                      │
                                                          有邮箱      │     无邮箱
                                                          直接用      │     或为空
                                                              │        │        │
                                                              │        │        ▼
                                                              │        │  ┌─────────────────────┐
                                                              │        │  │ 额外调用              │
                                                              │        │  │ GET /user/emails      │
                                                              │        │  │ 找 primary+verified   │
                                                              │        │  │ 的邮箱                │
                                                              │        │  └──────────┬──────────┘
                                                              │        │             │
                                                              ▼        ▼             ▼
                                                           ┌──────────────────────────────┐
                                                           │  得到 OAuthUserInfo:          │
                                                           │  - provider_user_id (GitHub ID)│
                                                           │  - email                      │
                                                           │  - display_name (login)       │
                                                           │  - avatar_url                 │
                                                           │  - raw_profile (完整 JSON)     │
                                                           └──────────────┬───────────────┘
                                                                          │
                                                                          ▼
```

## 用户查找/创建决策树

```
                     ┌──────────────────────────┐
                     │ 拿到 OAuthUserInfo        │
                     │ (provider + user_id +     │
                     │  email + name + avatar)   │
                     └────────────┬─────────────┘
                                  │
                                  ▼
                 ┌────────────────────────────────────┐
                 │ A. 查找已有 OAuth 绑定               │
                 │    SELECT * FROM oauth_accounts     │
                 │    WHERE provider = ?               │
                 │    AND provider_user_id = ?         │
                 └──────────────┬─────────────────────┘
                                │
                        ┌───────┴───────┐
                        │               │
                    找到绑定         未找到绑定
                        │               │
                        ▼               ▼
            ┌─────────────────┐  ┌──────────────────────────────┐
            │ 已绑定用户！     │  │ B. 检查 state.user_id         │
            │                 │  │    (发起时已登录用户传入)       │
            │ 1. 查找本地用户  │  │    → 表示"绑定"操作            │
            │ 2. 更新 OAuth   │  └──────────────┬───────────────┘
            │    绑定信息      │                 │
            │    (token/profile│          ┌──────┴──────┐
            │     等刷新)      │          │             │
            │ 3. 签发 JWT +   │      有 user_id    无 user_id
            │    refresh_token│          │             │
            │ 4. 302 重定向   │          ▼             ▼
            │    到前端        │  ┌─────────────┐ ┌──────────────────┐
            └─────────────────┘  │ C. 绑定模式 │ │ D. 检查邮箱匹配   │
                                 │             │ │                  │
                                 │ 1. 绑定到   │ │ 有 email 且      │
                                 │    当前用户 │ │ 能在 users 表    │
                                 │ 2. 签发 JWT │ │ 找到匹配邮箱？   │
                                 │ 3. 重定向   │ └────────┬─────────┘
                                 └──────┬──────┘          │
                                        │          ┌──────┴──────┐
                                        │          │             │
                                        │     找到匹配用户   无匹配用户
                                        │          │             │
                                        │          ▼             ▼
                                        │  ┌──────────────┐ ┌──────────────────────┐
                                        │  │ E. 自动绑定   │ │ F. 自动注册新用户     │
                                        │  │              │ │                      │
                                        │  │ 1. 将 OAuth  │ │ 1. 生成唯一 username  │
                                        │  │    绑定到    │ │    base = login 名    │
                                        │  │    匹配用户  │ │    sanitize 清理      │
                                        │  │ 2. 签发 JWT  │ │    冲突→加前缀/后缀   │
                                        │  │ 3. 重定向    │ │                      │
                                        │  └──────┬───────┘ │ 2. 生成占位密码       │
                                        │         │         │    "!oauth:github:xxx"│
                                        │         │         │                      │
                                        │         │         │ 3. INSERT INTO users  │
                                        │         │         │    role = 'reader'    │
                                        │         │         │                      │
                                        │         │         │ 4. UPDATE users SET   │
                                        │         │         │    avatar = 头像URL   │
                                        │         │         │    email_verified = 1 │
                                        │         │         │                      │
                                        │         │         │ 5. 发射 UserRegistered│
                                        │         │         │    事件               │
                                        │         │         │                      │
                                        │         │         │ 6. 创建 OAuth 绑定    │
                                        │         │         └──────────┬───────────┘
                                        │         │                    │
                                        ▼         ▼                    ▼
                                        ┌──────────────────────────────────┐
                                        │     签发 JWT + refresh_token      │
                                        │                                  │
                                        │  1. generate_access_token_internal│
                                        │     (user_id, role, tenant_id,    │
                                        │      jwt_secret, expires_in)      │
                                        │                                  │
                                        │  2. generate_refresh_token_string │
                                        │     (32字节随机 hex)              │
                                        │                                  │
                                        │  3. INSERT INTO refresh_tokens    │
                                        │                                  │
                                        │  4. 302 重定向到前端:              │
                                        │     {redirect_url}                │
                                        │     ?access_token=xxx             │
                                        │     &refresh_token=xxx            │
                                        │     &expires_in=900               │
                                        └──────────────────────────────────┘
```

## 用户名自动生成策略

```
输入: display_name = "octocat"
      │
      ▼
┌─────────────────────┐
│ sanitize_username()  │
│ 只保留 a-z A-Z 0-9 _ │
│ 去除首尾下划线        │
└──────────┬──────────┘
           │
           ▼
    username = "octocat"
           │
           ▼
┌─────────────────────────────┐
│ SELECT * FROM users          │
│ WHERE username = 'octocat'   │
└──────────────┬──────────────┘
               │
        ┌──────┴──────┐
        │             │
    不存在(OK)     已存在
        │             │
        ▼             ▼
  返回 "octocat"  ┌────────────────────────────────┐
                  │ 尝试加 GitHub 前缀              │
                  │ username = "github_octocat"     │
                  │ SELECT ... WHERE username = ?   │
                  └──────────────┬─────────────────┘
                                 │
                          ┌──────┴──────┐
                          │             │
                      不存在(OK)     已存在
                          │             │
                          ▼             ▼
                   返回               ┌──────────────────────────┐
                   "github_octocat"   │ 追加 4 位随机 hex 后缀    │
                                      │ suffix = random_hex(2)   │
                                      │ "github_octocat_a3f8"    │
                                      └──────────────────────────┘
```

## 安全机制

```
┌──────────────────────────────────────────────────────────────────┐
│                        安全防护层                                 │
├──────────────────────────────────────────────────────────────────┤
│                                                                  │
│  ┌──────────── PKCE (Proof Key for Code Exchange) ────────────┐ │
│  │                                                             │ │
│  │  客户端(后端)              GitHub                           │ │
│  │  ┌─────────────┐          ┌──────────┐                     │ │
│  │  │ 生成随机     │          │          │                     │ │
│  │  │ code_verifier│────────>│ 存储     │                     │ │
│  │  │ (43 字符)    │          │ code_    │                     │ │
│  │  └──────┬──────┘          │ challenge│                     │ │
│  │         │                 │          │                     │ │
│  │         ▼                 │          │                     │ │
│  │  SHA256(code_verifier)    │          │                     │ │
│  │         │                 │          │                     │ │
│  │         ▼                 │          │                     │ │
│  │  BASE64URL(hash)          │          │                     │ │
│  │  = code_challenge         │          │                     │ │
│  │                           │          │                     │ │
│  │  授权请求 ───────────────>│ 校验     │                     │ │
│  │  带 code_challenge        │ challenge│                     │ │
│  │                           │          │                     │ │
│  │  回调时 ─────────────────>│ 用       │                     │ │
│  │  带 code_verifier         │ verifier │                     │ │
│  │                           │ 重新计算 │                     │ │
│  │                           │ 验证匹配 │                     │ │
│  │                           └──────────┘                     │ │
│  │                                                             │ │
│  │  防止：authorization code 被截获后无法使用                    │ │
│  └─────────────────────────────────────────────────────────────┘ │
│                                                                  │
│  ┌──────────── State 参数 (CSRF 防护) ────────────────────────┐ │
│  │                                                             │ │
│  │  发起时:                                                    │ │
│  │    state = random_hex(32)   →  64 字符随机串               │ │
│  │    INSERT INTO oauth_states (id=state, ..., expires_at)    │ │
│  │                                                             │ │
│  │  回调时:                                                    │ │
│  │    SELECT * FROM oauth_states WHERE id = ?                 │ │
│  │    AND expires_at > datetime('now')                        │ │
│  │    DELETE FROM oauth_states WHERE id = ?  ← 一次性使用     │ │
│  │                                                             │ │
│  │  防止：CSRF 攻击（伪造回调请求）                             │ │
│  └─────────────────────────────────────────────────────────────┘ │
│                                                                  │
│  ┌──────────── Rate Limiting ─────────────────────────────────┐ │
│  │                                                             │ │
│  │  OAuth 端点受全局 rate_limit 保护                           │ │
│  │  默认 60 次/分钟/IP                                         │ │
│  └─────────────────────────────────────────────────────────────┘ │
│                                                                  │
│  ┌──────────── 解绑安全 ──────────────────────────────────────┐ │
│  │                                                             │ │
│  │  解绑前检查:                                                │ │
│  │    if user.password_hash 是占位值 (以 "!oauth:" 开头)       │ │
│  │       AND oauth 绑定数量 <= 1                               │ │
│  │    then 拒绝解绑                                           │ │
│  │    → 防止用户无法登录                                       │ │
│  └─────────────────────────────────────────────────────────────┘ │
│                                                                  │
└──────────────────────────────────────────────────────────────────┘
```

## 数据库表关系

```
┌──────────────────────┐       ┌──────────────────────────┐
│       users          │       │     oauth_accounts        │
├──────────────────────┤       ├──────────────────────────┤
│ id TEXT PK           │◄──┐   │ id TEXT PK               │
│ tenant_id TEXT       │   │   │ user_id TEXT FK ─────────┤──┘
│ email TEXT UNIQUE    │   │   │ provider TEXT            │    一个用户可绑定
│ username TEXT UNIQUE │   │   │ provider_user_id TEXT    │    多个 Provider
│ password_hash TEXT   │   │   │ email TEXT               │
│ role TEXT            │   │   │ display_name TEXT        │
│ avatar TEXT          │   │   │ avatar_url TEXT          │
│ bio TEXT             │   │   │ access_token TEXT        │
│ website TEXT         │   │   │ refresh_token TEXT       │
│ email_verified INT   │   │   │ token_expires_at TEXT   │
│ created_at TEXT      │   │   │ profile TEXT (JSON)      │
│ updated_at TEXT      │   │   │ created_at TEXT          │
└──────────────────────┘   │   │ updated_at TEXT          │
                           │   └──────────────────────────┘
                           │
                           │   UNIQUE(provider, provider_user_id)
                           │
                           │   ┌──────────────────────────┐
                           │   │     oauth_states          │
                           │   ├──────────────────────────┤
                           │   │ id TEXT PK (= state 值)   │
                           │   │ provider TEXT             │
                           └───│ user_id TEXT FK (可空)    │  ← 绑定模式传入
                               │ code_verifier TEXT        │
                               │ created_at TEXT           │
                               │ expires_at TEXT           │
                               │                          │
                               │ 10 分钟后过期             │
                               │ Worker cron 定期清理      │
                               └──────────────────────────┘
```

## 前端对接示例

```
┌────────────────────────────────────────────────────────────────┐
│  登录页 (/login)                                               │
│                                                                │
│  ┌─────────────────────────────────┐                          │
│  │  邮箱: [________________]        │                          │
│  │  密码: [________________]        │                          │
│  │  [ 登 录 ]                       │                          │
│  │                                  │                          │
│  │  ────── 或 ──────                │                          │
│  │                                  │                          │
│  │  [GitHub 登录]  [Google 登录]     │  ← 调用 GET /api/v1/   │
│  │                                  │     auth/oauth/github   │
│  └─────────────────────────────────┘     (浏览器直接跳转)      │
│                                                                │
├────────────────────────────────────────────────────────────────┤
│  回调页 (/auth/callback)                                       │
│                                                                │
│  useEffect(() => {                                             │
│    const params = new URLSearchParams(location.search);        │
│    const accessToken = params.get('access_token');             │
│    const refreshToken = params.get('refresh_token');           │
│    const expiresIn = params.get('expires_in');                 │
│                                                                │
│    if (accessToken) {                                          │
│      localStorage.setItem('access_token', accessToken);       │
│      localStorage.setItem('refresh_token', refreshToken);     │
│      navigate('/');                                            │
│    } else {                                                    │
│      // 显示错误                                                │
│      const error = params.get('error');                        │
│      showToast(error || 'OAuth login failed');                 │
│      navigate('/login');                                       │
│    }                                                           │
│  }, []);                                                       │
│                                                                │
├────────────────────────────────────────────────────────────────┤
│  账号设置页 (/settings/account)                                 │
│                                                                │
│  已绑定的社交账号:                                               │
│  ┌──────────────────────────────────────┐                     │
│  │ 🐙 GitHub  octocat   [解绑]          │  ← DELETE /api/v1/  │
│  │ 🔵 Google  (未绑定)   [绑定]          │     auth/oauth/     │
│  │ 💬 微信    (未绑定)   [绑定]          │     github/unbind   │
│  └──────────────────────────────────────┘                     │
│                                                                │
│  [设置密码]  ← OAuth 用户首次设置密码后成为"混合登录"用户       │
│                                                                │
└────────────────────────────────────────────────────────────────┘
```

## JWT Token 签发流程（OAuth 和普通登录共用）

```
                        ┌─────────────────────┐
                        │   确认用户身份       │
                        │   user_id + role     │
                        │   + tenant_id        │
                        └──────────┬──────────┘
                                   │
                                   ▼
                ┌──────────────────────────────────────┐
                │  生成 Access Token (JWT HS256)        │
                │                                      │
                │  Claims:                             │
                │  {                                   │
                │    "sub": "0192a3b4-user-id",        │
                │    "role": "reader",                 │
                │    "tenant_id": "default",           │
                │    "exp": 1713980000,                │
                │    "iat": 1713979100                 │
                │  }                                   │
                │                                      │
                │  签名: HMAC-SHA256(jwt_secret)        │
                │  有效期: 15 分钟 (可配置)              │
                └──────────────┬───────────────────────┘
                               │
                               ▼
                ┌──────────────────────────────────────┐
                │  生成 Refresh Token                   │
                │                                      │
                │  32 字节随机 → 64 字符 hex 字符串     │
                │  "a3f8b2c1d4e5f6...64chars"          │
                │                                      │
                │  INSERT INTO refresh_tokens           │
                │  有效期: 7 天 (可配置)                 │
                └──────────────┬───────────────────────┘
                               │
                               ▼
                ┌──────────────────────────────────────┐
                │  返回给前端 (通过 URL 参数)            │
                │                                      │
                │  {redirect_url}                      │
                │    ?access_token=eyJhbG...            │
                │    &refresh_token=a3f8b2c1...         │
                │    &expires_in=900                    │
                └──────────────────────────────────────┘
```
