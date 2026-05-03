# GitHub OAuth 接入详细步骤

## 1. 创建 GitHub OAuth App

### 1.1 进入 GitHub 开发者设置

1. 登录 GitHub
2. 进入 **Settings** → **Developer settings** → **OAuth Apps**
3. 直接链接：https://github.com/settings/developers

### 1.2 注册新应用

点击 **New OAuth App**，填写以下信息：

| 字段 | 本地开发值 | 生产环境值 |
|------|-----------|-----------|
| Application name | `raisfast-dev` | `你的产品名` |
| Homepage URL | `http://localhost:3000` | `https://yourdomain.com` |
| Authorization callback URL | `http://localhost:9000/api/v1/auth/oauth/github/callback` | `https://api.yourdomain.com/api/v1/auth/oauth/github/callback` |
| Application description | (可选) | (可选) |

> **注意**：Callback URL 必须与后端路由完全匹配，包括协议（http/https）。

点击 **Register application**。

### 1.3 获取凭证

注册完成后，在应用详情页：

1. 记下 **Client ID**（形如 `Ov23lixxxxxxxxxx`）
2. 点击 **Generate a new client secret**
3. 记下 **Client Secret**（只显示一次，务必保存）

> 如果忘记 Secret，可以重新生成，旧的会立即失效。

## 2. 配置后端环境变量

编辑项目根目录 `.env` 文件：

```env
# ── OAuth2 社交登录 ──────────────────────────────────────────
OAUTH_ENABLED=true
OAUTH_REDIRECT_URL=http://localhost:3000/auth/callback

# GitHub OAuth
OAUTH_GITHUB_CLIENT_ID=Ov23liwYwANw8fvQVhI0
OAUTH_GITHUB_CLIENT_SECRET=42ab89916b6731b3ead0dbfadca581aecbb6cb60
```

### 配置项说明

| 环境变量 | 必填 | 默认值 | 说明 |
|---------|------|--------|------|
| `OAUTH_ENABLED` | 是 | `false` | 总开关，必须设为 `true` |
| `OAUTH_REDIRECT_URL` | 否 | `http://localhost:3000/auth/callback` | 登录成功后 302 重定向到前端的地址 |
| `OAUTH_GITHUB_CLIENT_ID` | 是 | - | GitHub OAuth App 的 Client ID |
| `OAUTH_GITHUB_CLIENT_SECRET` | 是 | - | GitHub OAuth App 的 Client Secret |

> **修改 `.env` 后必须重启后端进程**才能生效。

## 3. 运行数据库迁移

OAuth 功能需要 3 张新表：

| 表名 | 用途 |
|------|------|
| `oauth_accounts` | 存储 OAuth 绑定关系 |
| `oauth_states` | 存储 PKCE state（短期，10 分钟过期） |
| `users` (改动) | 新增 `email_verified` 字段 |

迁移文件：`migrations/019_oauth.sql`

首次启动后端时自动执行，也可以手动运行：

```bash
cargo sqlx migrate run --features "db-sqlite"
```

## 4. 启动后端

```bash
SWAGGER_UI_DOWNLOAD_URL=file:///tmp/swagger-ui.zip cargo run --features "db-sqlite"
```

启动成功后，日志中应看到：

```
OAuth provider registered: github
```

如果没有看到这行，说明 `OAUTH_GITHUB_CLIENT_ID` 或 `OAUTH_GITHUB_CLIENT_SECRET` 未正确配置。

## 5. 验证配置

```bash
# 查看已配置的 OAuth Provider
curl http://localhost:9000/api/v1/auth/oauth/providers

# 预期返回：
# {"code":0,"message":"操作成功","data":[{"name":"github","configured":true}]}
```

## 6. 测试完整流程

### 6.1 发起 OAuth 登录

在浏览器中打开：

```
http://localhost:9000/api/v1/auth/oauth/github
```

预期行为：302 重定向到 GitHub 授权页面，URL 形如：

```
https://github.com/login/oauth/authorize
  ?client_id=Ov23liwYwANw8fvQVhI0
  &state=a3f8b2c1d4e5...（64字符随机串）
  &code_challenge=E9Melhoa2OwvFrEM...（PKCE challenge）
  &code_challenge_method=S256
  &scope=user:email
```

### 6.2 在 GitHub 授权

点击 **Authorize** 按钮。

### 6.3 GitHub 回调

GitHub 将用户重定向到：

```
http://localhost:9000/api/v1/auth/oauth/github/callback?code=xxx&state=xxx
```

后端处理流程：

1. 校验 `state`（防 CSRF，一次性使用）
2. 用 `code` + PKCE `code_verifier` 换 access_token
3. 获取 GitHub 用户信息（`/user` + `/user/emails`）
4. 查找/创建本地用户
5. 签发 JWT

### 6.4 重定向到前端

处理完成后，302 重定向到：

```
http://localhost:3000/auth/callback
  ?access_token=eyJhbGciOiJIUzI1NiIs...
  &refresh_token=a3f8b2c1d4e5f6...
  &expires_in=900
```

### 6.5 curl 测试（不跟随重定向）

```bash
# 测试发起授权
curl -v http://localhost:9000/api/v1/auth/oauth/github 2>&1 | grep "Location:"

# 应输出类似：
# Location: https://github.com/login/oauth/authorize?client_id=...
```

## 7. 前端对接

### 7.1 登录页添加 GitHub 按钮

```tsx
// web/src/app/login/page.tsx
const API_BASE = "http://localhost:9000/api/v1";

<a href={`${API_BASE}/auth/oauth/github`}>
  <GitHubIcon />
  GitHub 登录
</a>
```

> 直接用 `<a href>` 跳转，不需要 `fetch`。浏览器会跟随 302 重定向到 GitHub。

### 7.2 创建 OAuth 回调页

```tsx
// web/src/app/auth/callback/page.tsx
"use client";

import { useEffect } from "react";
import { useRouter } from "next/navigation";

export default function OAuthCallback() {
  const router = useRouter();

  useEffect(() => {
    const params = new URLSearchParams(window.location.search);
    const accessToken = params.get("access_token");
    const refreshToken = params.get("refresh_token");
    const expiresIn = params.get("expires_in");
    const error = params.get("error");

    if (error) {
      console.error("OAuth error:", error);
      router.push("/login?error=oauth_failed");
      return;
    }

    if (accessToken) {
      localStorage.setItem("access_token", accessToken);
      localStorage.setItem("refresh_token", refreshToken || "");
      router.push("/");
    } else {
      router.push("/login?error=no_token");
    }
  }, [router]);

  return <div>正在登录...</div>;
}
```

### 7.3 账号设置页 — 绑定/解绑

```tsx
// 查看已绑定 Provider
const { data } = useQuery({
  queryKey: ["oauth-bindings"],
  queryFn: () =>
    fetch("http://localhost:9000/api/v1/auth/oauth/bindings", {
      headers: { Authorization: `Bearer ${token}` },
    }).then((r) => r.json()),
});

// 解绑
const unbind = async (provider: string) => {
  await fetch(`http://localhost:9000/api/v1/auth/oauth/${provider}/unbind`, {
    method: "DELETE",
    headers: { Authorization: `Bearer ${token}` },
  });
};
```

## 8. API 端点完整列表

| 方法 | 路径 | 认证 | 说明 |
|------|------|------|------|
| GET | `/api/v1/auth/oauth/{provider}` | 无 | 发起 OAuth 登录，302 到 Provider |
| GET | `/api/v1/auth/oauth/{provider}/callback` | 无 | Provider 回调处理 |
| GET | `/api/v1/auth/oauth/providers` | 无 | 已配置的 Provider 列表 |
| GET | `/api/v1/auth/oauth/bindings` | 需登录 | 当前用户绑定列表 |
| DELETE | `/api/v1/auth/oauth/{provider}/unbind` | 需登录 | 解绑指定 Provider |

## 9. 用户场景说明

### 场景 A：首次 GitHub 登录（自动注册）

```
1. 用户点击 "GitHub 登录"
2. GitHub 授权
3. 后端发现该 GitHub ID 未绑定任何本地用户
4. 自动创建本地用户：
   - username = GitHub login 名（冲突时追加前缀/随机后缀）
   - email = GitHub 主邮箱
   - password = 占位符（"!oauth:github:xxx"）
   - role = "reader"
   - avatar = GitHub 头像
   - email_verified = 1
5. 创建 oauth_accounts 绑定
6. 签发 JWT，重定向到前端
```

### 场景 B：已有账号，邮箱匹配自动绑定

```
1. 用户已用 email 注册（如 alice@gmail.com）
2. GitHub 账号主邮箱也是 alice@gmail.com
3. 首次用 GitHub 登录
4. 后端发现邮箱匹配已有用户
5. 自动创建 oauth_accounts 绑定
6. 签发 JWT，重定向到前端
```

### 场景 C：再次 GitHub 登录（已有绑定）

```
1. 用户再次点击 "GitHub 登录"
2. GitHub 授权
3. 后端在 oauth_accounts 找到已有绑定
4. 更新 GitHub token/profile 信息
5. 签发 JWT，重定向到前端
```

### 场景 D：已登录用户绑定 GitHub

```
1. 用户在账号设置页点击 "绑定 GitHub"
2. 前端带 token 跳转 /api/v1/auth/oauth/github
   （后端从 JWT 提取 user_id，存入 oauth_states）
3. GitHub 授权
4. 回调时后端发现 state.user_id 存在
5. 创建 oauth_accounts 绑定到该用户
6. 签发 JWT，重定向到前端
```

### 场景 E：解绑

```
1. 用户在账号设置页点击 "解绑 GitHub"
2. DELETE /api/v1/auth/oauth/github/unbind
3. 后端检查安全性：
   - 如果用户没有密码（纯 OAuth 用户）且只剩这一个绑定 → 拒绝
   - 否则 → 删除绑定
```

## 10. 安全机制

### PKCE（Proof Key for Code Exchange）

每次授权请求：

```
1. 生成 code_verifier（43 字符随机串）
2. code_challenge = BASE64URL(SHA256(code_verifier))
3. 授权 URL 携带 code_challenge + method=S256
4. 回调时发送 code_verifier
5. GitHub 用 SHA256 验证匹配
```

防止 authorization code 被截获后滥用。

### State 参数（CSRF 防护）

```
1. 生成 state（64 字符随机 hex）
2. 存入 oauth_states 表（10 分钟过期）
3. 授权 URL 携带 state
4. 回调时校验 state 匹配
5. 用后即删（一次性）
```

### 解绑安全

无密码的纯 OAuth 用户仅剩一个绑定时，拒绝解绑，防止用户无法登录。

## 11. 生产环境部署

### 必须修改的配置

```env
# .env.production
OAUTH_ENABLED=true
OAUTH_REDIRECT_URL=https://yourdomain.com/auth/callback

# 强密钥
JWT_SECRET=至少32字符的随机字符串

# CORS
CORS_ORIGINS=https://yourdomain.com
```

### GitHub OAuth App 配置更新

在 GitHub OAuth App 设置中更新：

| 字段 | 值 |
|------|-----|
| Homepage URL | `https://yourdomain.com` |
| Authorization callback URL | `https://api.yourdomain.com/api/v1/auth/oauth/github/callback` |

> 如果前后端同域（通过 nginx 反向代理），callback URL 可以是：
> `https://yourdomain.com/api/v1/auth/oauth/github/callback`

### Nginx 配置示例

```nginx
server {
    listen 443 ssl;
    server_name yourdomain.com;

    # 前端
    location / {
        proxy_pass http://127.0.0.1:3000;
    }

    # 后端 API
    location /api/ {
        proxy_pass http://127.0.0.1:9000;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }
}
```

## 12. 故障排查

### "OAuth is not enabled"

```
检查：
1. .env 中 OAUTH_ENABLED=true（无 # 前缀）
2. 重启了后端进程
3. 进程启动日志中有 "OAuth provider registered: github"
```

### "OAuth provider 'github' is not configured"

```
检查：
1. OAUTH_GITHUB_CLIENT_ID 和 OAUTH_GITHUB_CLIENT_SECRET 都已设置
2. 值不为空
3. 重启了后端进程
```

### GitHub 回调报 400 "invalid or expired OAuth state"

```
可能原因：
1. state 超过 10 分钟过期（用户在 GitHub 页面停留太久）
2. state 被重复使用（浏览器刷新了回调 URL）
3. oauth_states 表未创建（迁移未执行）

解决：重新发起授权（重新访问 /api/v1/auth/oauth/github）
```

### GitHub 回调报 "GitHub token exchange failed"

```
可能原因：
1. Client Secret 不正确
2. Authorization code 已过期（10 分钟有效期）
3. GitHub OAuth App 的 callback URL 不匹配

解决：
1. 在 GitHub App 设置页重新生成 Secret
2. 确认 callback URL 完全匹配
```

### 邮箱未自动获取

```
GitHub API 只返回 public 邮箱。私有邮箱需要：
1. 授权时 scope=user:email（已包含）
2. 额外调用 GET /user/emails API（已自动处理）

如果仍然没有邮箱，检查：
1. GitHub 账号是否设置了主邮箱
2. 邮箱是否已验证
```

### 重定向到前端后 token 无效

```
检查：
1. OAUTH_REDIRECT_URL 是否正确
2. JWT_SECRET 是否在重启后变了
3. 前端是否正确提取 URL 参数
```
