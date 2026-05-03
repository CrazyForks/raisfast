# Google OAuth 接入详细步骤

## 1. 创建 Google Cloud 项目

### 1.1 进入 Google Cloud Console

1. 打开 https://console.cloud.google.com/
2. 使用 Google 账号登录

### 1.2 创建新项目

1. 点击顶部导航栏的项目选择器
2. 点击 **NEW PROJECT**
3. 填写：
   - Project name: `raisfast`
   - Organization: 保持默认
4. 点击 **CREATE**
5. 等待项目创建完成，选择该项目

## 2. 配置 OAuth 同意屏幕

> Google 要求所有 OAuth 应用必须配置同意屏幕，否则无法使用。

### 2.1 进入配置页面

1. 左侧菜单 → **APIs & Services** → **OAuth consent screen**
2. 直接链接：https://console.cloud.google.com/apis/credentials/consent

### 2.2 选择用户类型

| 用户类型 | 适用场景 |
|---------|---------|
| External | 任何 Google 账号都可登录（推荐） |
| Internal | 仅组织内 G Suite 账号可登录 |

选择 **External** → 点击 **CREATE**

### 2.3 填写应用信息

**OAuth consent screen 页面：**

| 字段 | 值 |
|------|-----|
| App name | `raisfast` |
| User support email | 你的邮箱 |
| App logo | (可选，后续上传) |

**App domain（全部可选）：**

| 字段 | 值 |
|------|-----|
| Application home page | `http://localhost:3000` |
| Application privacy policy link | (可选) |
| Application terms of service link | (可选) |

**Authorized domains：**

点击 **ADD DOMAIN**，添加：

| 域名 |
|------|
| `localhost` |

> 生产环境添加你的域名如 `yourdomain.com`

**Developer contact information：**

| 字段 | 值 |
|------|-----|
| Email addresses | 你的邮箱 |

点击 **SAVE AND CONTINUE**

### 2.4 配置 Scopes

1. 点击 **ADD OR REMOVE SCOPES**
2. 搜索并勾选以下 scope：

| Scope | 说明 |
|-------|------|
| `.../auth/userinfo.email` | 读取用户邮箱 |
| `.../auth/userinfo.profile` | 读取用户基本信息（名字、头像） |
| `openid` | OpenID Connect |

3. 点击 **UPDATE** → **SAVE AND CONTINUE**

### 2.5 添加测试用户（发布前必须）

如果应用状态为 **Testing**，只有添加的测试用户可以登录。

1. 点击 **ADD USERS**
2. 输入你的 Gmail 邮箱
3. 点击 **ADD** → **SAVE AND CONTINUE**

### 2.6 发布应用（可选）

Testing 状态下只有测试用户能登录。如需所有人可登录：

1. 回到 OAuth consent screen
2. 点击 **PUBLISH APP**
3. 确认发布

> **注意**：发布后如果请求敏感 scope 可能需要 Google 审核。`email` + `profile` 是非敏感的，无需审核。

## 3. 创建 OAuth 2.0 凭证

### 3.1 进入凭证页面

1. 左侧菜单 → **APIs & Services** → **Credentials**
2. 直接链接：https://console.cloud.google.com/apis/credentials

### 3.2 创建 OAuth 客户端

1. 点击顶部 **+ CREATE CREDENTIALS** → **OAuth client ID**
2. 填写：

| 字段 | 值 |
|------|-----|
| Application type | **Web application** |
| Name | `raisfast-web` |

**Authorized JavaScript origins：**

点击 **ADD URI**，添加：

| 环境 | URI |
|------|-----|
| 本地开发 | `http://localhost:9000` |
| 生产环境 | `https://yourdomain.com` |

**Authorized redirect URIs：**

点击 **ADD URI**，添加：

| 环境 | URI |
|------|-----|
| 本地开发 | `http://localhost:9000/api/v1/auth/oauth/google/callback` |
| 生产环境 | `https://yourdomain.com/api/v1/auth/oauth/google/callback` |

3. 点击 **CREATE**

### 3.3 获取凭证

创建完成后弹出窗口显示：

- **Your Client ID**：形如 `123456789-abcxxx.apps.googleusercontent.com`
- **Your Client Secret**：形如 `GOCSPX-xxxxxxxxxx`

点击 **DOWNLOAD JSON** 备份凭证（可选）。

> Client Secret 可以在凭证详情页随时查看，不会像 GitHub 那样只显示一次。

## 4. 后端代码实现

Google OAuth 需要新增 Provider 实现。创建 `src/oauth/google.rs`：

### 4.1 Google Provider 实现

Google 的 OAuth2 端点：

| 端点 | URL |
|------|-----|
| Authorization | `https://accounts.google.com/o/oauth2/v2/auth` |
| Token Exchange | `https://oauth2.googleapis.com/token` |
| User Info | `https://www.googleapis.com/oauth2/v2/userinfo` |

### 4.2 关键差异（对比 GitHub）

| 对比项 | GitHub | Google |
|--------|--------|--------|
| Scope | `user:email` | `openid email profile` |
| Token 响应 | `access_token` 在 JSON body | 同 |
| 用户信息 API | `GET /user` + `GET /user/emails` | `GET /userinfo`（一次请求） |
| 用户 ID | 数字字符串 | 数字字符串（`sub` 字段） |
| 邮箱 | 可能需要额外请求 | 直接返回，有 `verified_email` 字段 |

### 4.3 实现步骤

#### 步骤一：创建 Google Provider 文件

在 `src/oauth.rs` 中添加 Google Provider（与 GitHub 同文件）：

```rust
// 在 src/oauth.rs 末尾添加

// ── Google Provider ────────────────────────────────────────────

pub struct GoogleProvider {
    client_id: String,
    client_secret: String,
}

impl GoogleProvider {
    pub fn new(client_id: String, client_secret: String) -> Self {
        Self { client_id, client_secret }
    }
}

#[async_trait::async_trait]
impl OAuthProvider for GoogleProvider {
    fn name(&self) -> &str {
        "google"
    }

    fn authorize_url(&self, state: &str, code_challenge: &str) -> String {
        format!(
            "https://accounts.google.com/o/oauth2/v2/auth?client_id={}&state={}&code_challenge={}&code_challenge_method=S256&scope=openid+email+profile&response_type=code&access_type=offline",
            self.client_id, state, code_challenge
        )
    }

    async fn exchange_code(&self, code: &str, code_verifier: &str)
        -> AppResult<OAuthTokenResponse>
    {
        let client = reqwest::Client::new();
        let resp = client
            .post("https://oauth2.googleapis.com/token")
            .header("Accept", "application/json")
            .json(&serde_json::json!({
                "client_id": self.client_id,
                "client_secret": self.client_secret,
                "code": code,
                "code_verifier": code_verifier,
                "grant_type": "authorization_code",
                "redirect_uri": "",  // PKCE 模式不需要
            }))
            .send()
            .await
            .map_err(|e| AppError::Internal(
                anyhow::anyhow!("Google token exchange failed: {e}")
            ))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Internal(anyhow::anyhow!(
                "Google token exchange returned {status}: {body}"
            )));
        }

        resp.json::<OAuthTokenResponse>().await.map_err(|e| {
            AppError::Internal(anyhow::anyhow!(
                "Google token response parse failed: {e}"
            ))
        })
    }

    async fn fetch_user_info(&self, access_token: &str)
        -> AppResult<OAuthUserInfo>
    {
        let client = reqwest::Client::new();
        let resp = client
            .get("https://www.googleapis.com/oauth2/v2/userinfo")
            .header("Authorization", format!("Bearer {access_token}"))
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| AppError::Internal(
                anyhow::anyhow!("Google user info request failed: {e}")
            ))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Internal(anyhow::anyhow!(
                "Google user info returned {status}: {body}"
            )));
        }

        let profile: serde_json::Value = resp.json().await.map_err(|e| {
            AppError::Internal(anyhow::anyhow!(
                "Google user info parse failed: {e}"
            ))
        })?;

        let provider_user_id = profile["sub"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let email = profile["email"].as_str().map(|s| s.to_string());
        let display_name = profile["name"].as_str()
            .or_else(|| profile["given_name"].as_str())
            .map(|s| s.to_string());
        let avatar_url = profile["picture"].as_str().map(|s| s.to_string());

        Ok(OAuthUserInfo {
            provider_user_id,
            email,
            display_name,
            avatar_url,
            raw_profile: profile,
        })
    }
}
```

#### 步骤二：在 server.rs 注册 Google Provider

在 `build_oauth_registry` 函数中添加：

```rust
if let Some(google) = &config.oauth.google {
    registry.register(Box::new(
        crate::oauth::GoogleProvider::new(
            google.client_id.clone(),
            google.client_secret.clone(),
        ),
    ));
    tracing::info!("OAuth provider registered: google");
}
```

#### 步骤三：更新用户名生成策略

`ensure_unique_username` 中的 fallback 前缀需要支持 google：

```rust
async fn ensure_unique_username(pool: &crate::db::Pool, base: &str, provider: &str) -> AppResult<String> {
    // ...
    let prefixed = format!("{provider}_{username}");
    // ...
}
```

## 5. 配置环境变量

编辑 `.env`：

```env
# Google OAuth
OAUTH_GOOGLE_CLIENT_ID=123456789-abcxxx.apps.googleusercontent.com
OAUTH_GOOGLE_CLIENT_SECRET=GOCSPX-xxxxxxxxxx
```

> `OAUTH_ENABLED=true` 已在 GitHub 配置时设置，无需重复。

## 6. 验证配置

重启后端，检查日志：

```
OAuth provider registered: github
OAuth provider registered: google
```

测试 API：

```bash
# 查看已配置 Provider
curl http://localhost:9000/api/v1/auth/oauth/providers

# 预期返回：
# {"code":0,"message":"操作成功","data":[
#   {"name":"github","configured":true},
#   {"name":"google","configured":true}
# ]}

# 测试 Google 授权 URL
curl -v http://localhost:9000/api/v1/auth/oauth/google 2>&1 | grep "Location:"

# 应输出：
# Location: https://accounts.google.com/o/oauth2/v2/auth?client_id=...
```

## 7. 测试完整流程

### 7.1 发起 Google 登录

浏览器打开：

```
http://localhost:9000/api/v1/auth/oauth/google
```

预期：302 到 Google 登录页面

### 7.2 Google 授权

1. 选择 Google 账号
2. 如果是 Testing 模式，会看到 "This app isn't verified" 警告
3. 点击 **Advanced** → **Go to raisfast (unsafe)**
4. 点击 **Continue** 授权

### 7.3 回调处理

Google 回调到：

```
http://localhost:9000/api/v1/auth/oauth/google/callback?code=xxx&state=xxx&scope=email+profile+openid
```

后端处理后 302 到前端：

```
http://localhost:3000/auth/callback?access_token=...&refresh_token=...&expires_in=900
```

### 7.4 前端登录页添加 Google 按钮

```tsx
<a href={`${API_BASE}/auth/oauth/google`}>
  <GoogleIcon />
  Google 登录
</a>
```

## 8. 常见问题

### "This app isn't verified" 警告

**原因**：应用处于 Testing 状态，且请求了非敏感 scope。

**解决（开发阶段）**：
1. 确保你的邮箱已添加到 Test Users
2. 点击 **Advanced** → **Go to raisfast (unsafe)** 即可继续

**解决（生产阶段）**：
1. 在 OAuth consent screen 页面点击 **PUBLISH APP**

### Error 400: redirect_uri_mismatch

**原因**：Google OAuth App 中配置的 redirect URI 与实际不匹配。

**检查**：
1. Google Console → Credentials → 你的 OAuth client → Authorized redirect URIs
2. 确保包含 `http://localhost:9000/api/v1/auth/oauth/google/callback`
3. 注意尾部斜杠、协议、端口号必须完全一致

### Error 401: invalid_client

**原因**：Client ID 或 Client Secret 不正确。

**解决**：
1. 复制 Google Console 中的完整 Client ID（很长的字符串）
2. 确认 `.env` 中没有多余空格或换行

### 获取不到邮箱

**检查**：
1. OAuth consent screen 中是否添加了 `email` scope
2. 授权 URL 中 `scope=openid+email+profile` 是否完整
3. Google 账号是否设置了邮箱

### 用户名冲突

如果 Google 用户名与已有用户冲突，系统自动追加前缀：

```
alice → google_alice → google_alice_a3f8
```

## 9. 生产环境部署

### 9.1 更新 Google OAuth App

| 配置项 | 添加值 |
|--------|--------|
| Authorized JavaScript origins | `https://yourdomain.com` |
| Authorized redirect URIs | `https://yourdomain.com/api/v1/auth/oauth/google/callback` |

### 9.2 发布应用

1. OAuth consent screen → **PUBLISH APP**
2. 如果 scope 中包含敏感 scope，需要提交 Google 审核（通常 1-3 天）

### 9.3 隐私政策页面

Google 要求应用提供隐私政策链接：

1. 在前端创建 `/privacy` 页面
2. 在 OAuth consent screen 中添加 Privacy policy URL

### 9.4 环境变量

```env
OAUTH_GOOGLE_CLIENT_ID=your-production-client-id
OAUTH_GOOGLE_CLIENT_SECRET=your-production-client-secret
```

## 10. Google vs GitHub 对比

| 对比项 | GitHub | Google |
|--------|--------|--------|
| 创建凭证 | Settings → Developer settings | Google Cloud Console |
| 凭证数量 | Client ID + Secret（同页面） | Client ID + Secret（同页面） |
| 回调 URL 配置 | 创建时设置，可修改 | 创建时设置，可修改 |
| Scope | `user:email` | `openid email profile` |
| 邮箱获取 | 可能需要额外 API 调用 | 直接返回 |
| 用户唯一标识 | `id`（数字） | `sub`（数字） |
| 头像 | `avatar_url` | `picture` |
| 昵称 | `login` | `name` 或 `given_name` |
| 应用审核 | 不需要 | 敏感 scope 需要审核 |
| 测试模式 | 无 | Testing 模式限制用户 |
| Consent screen | 简单 | 复杂（需完整配置） |
