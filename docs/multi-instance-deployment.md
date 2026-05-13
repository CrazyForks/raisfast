# 多实例部署方案：单机多租户

> 一台服务器运行多个 raisfast 进程，每个用户一个实例、一个 SQLite 文件，通过 Unix Socket + Caddy 反向代理实现隔离与路由。

---

## 1. 架构概览

```
用户浏览器
    │
    ▼
┌─────────────────────┐
│   Caddy 反向代理     │  ← 自动 HTTPS (Let's Encrypt)
│   *.api.example.com  │
└───┬─────┬─────┬─────┘
    │     │     │     Unix Socket
    ▼     ▼     ▼
  user1  user2  user3  ← 独立进程、独立 DB、独立配置
  .sock  .sock  .sock
```

**核心思路**：

- 同一个二进制文件，在不同配置下启动多次，每次是一个独立 OS 进程
- Linux 内核通过 mmap 共享只读代码段，60MB 二进制启动 30 个实例代码只占 ~50MB（一份）
- 每个实例只额外消耗私有内存（heap/stack/连接池等，约 25-35MB/实例）

---

## 2. 资源估算

### 2.1 单实例内存拆解

| 项目 | 大小 |
|------|------|
| 进程基础开销 | ~5 MB |
| Tokio runtime | ~3 MB |
| SQLite 连接池 + 缓存（cache_size=8MB） | ~10-20 MB |
| 请求处理 heap | ~5-10 MB |
| **合计** | **~25-35 MB** |

### 2.2 不同规格的容量

| 服务器 | 可用内存（去掉 OS） | 实例数（保守 30MB/个） |
|--------|---------------------|----------------------|
| 2 GB | ~1748 MB | ~56 个 |
| 4 GB | ~3748 MB | ~120 个 |
| 8 GB | ~7748 MB | ~250 个 |

### 2.3 真实瓶颈

| 瓶颈 | 说明 | 建议上限 |
|------|------|---------|
| 文件描述符 | 每实例 ~50-100 fd | `ulimit -n 65535` 后基本够 |
| CPU | 1-2 核上下文切换 | 单机 20-30 个活跃实例 |
| 磁盘 I/O | 多个 SQLite 同时写 WAL | 视磁盘性能而定 |
| 连接池 | 每实例独立，不能跨进程共享 | 每实例 `max_connections = 3` |

---

## 3. 配置文件

每个用户一个 TOML 配置文件：

```toml
# /etc/raisfast/users/user1.toml
database_url = "sqlite:/var/lib/raisfast/user1/data/raisfast.db"
listen = "unix:/run/raisfast/user1.sock"
data_dir = "/var/lib/raisfast/user1/data"
jwt_secret = "random-secret-for-user1"
```

```toml
# /etc/raisfast/users/user2.toml
database_url = "sqlite:/var/lib/raisfast/user2/data/raisfast.db"
listen = "unix:/run/raisfast/user2.sock"
data_dir = "/var/lib/raisfast/user2/data"
jwt_secret = "random-secret-for-user2"
```

**隔离项**：

| 配置项 | 说明 |
|--------|------|
| `database_url` | 每用户独立 SQLite 文件 |
| `listen` | 每用户独立 Unix socket |
| `data_dir` | 每用户独立数据目录 |
| `jwt_secret` | 每用户独立密钥 |

---

## 4. Rust 端监听 Unix Socket

```rust
use tokio::net::UnixListener;
use axum::Router;

async fn start_server(app: Router, listen: &str) -> anyhow::Result<()> {
    if let Some(path) = listen.strip_prefix("unix:") {
        // 清理旧 socket 文件，否则 bind 会失败
        let _ = std::fs::remove_file(path);
        let listener = UnixListener::bind(path)?;
        axum::serve(listener, app).await?;
    } else {
        // TCP fallback
        let listener = tokio::net::TcpListener::bind(listen).await?;
        axum::serve(listener, app).await?;
    }
    Ok(())
}
```

**注意**：启动时必须先 `remove_file` 清理旧 socket 文件，否则上一次非正常退出后 bind 会报 "Address already in use"。

---

## 5. systemd 管理多实例

使用 systemd **模板单元**（`@` 语法），一个 service 文件管理所有实例：

```ini
# /etc/systemd/system/raisfast@.service
[Unit]
Description=raisfast instance %i
After=network.target

[Service]
Type=simple
User=raisfast
Group=raisfast
ExecStart=/usr/local/bin/raisfast --config /etc/raisfast/users/%i.toml
RuntimeDirectory=raisfast
Restart=on-failure
LimitNOFILE=65535

[Install]
WantedBy=multi-user.target
```

### 5.1 常用命令

```bash
# 启动/停止单个实例
systemctl start raisfast@user1
systemctl stop raisfast@user2
systemctl restart raisfast@user3

# 查看状态
systemctl status raisfast@user1
journalctl -u raisfast@user1 -f

# 批量操作
systemctl start 'raisfast@*'
systemctl stop 'raisfast@*'
systemctl restart 'raisfast@*'

# 开机自启
systemctl enable raisfast@user1
```

---

## 6. 反向代理

### Caddy vs Nginx 对比

| | Caddy | Nginx |
|---|---|---|
| **自动 HTTPS** | 内置 Let's Encrypt，零配置 | 需要额外装 certbot + cron 续签 |
| **配置语法** | 简洁，接近自然语言 | 繁琐，指令多 |
| **动态路由** | 有 HTTP API，可运行时增删 | 需 reload，无原生 API |
| **性能** | 很好（略低于 Nginx） | 极致，行业标准 |
| **生态/成熟度** | 较新（2015） | 20+ 年历史，资料丰富 |
| **国内使用** | 知名度低 | 几乎所有运维都会 |

**选择建议**：快速迭代 / SaaS 自动化选 Caddy（自动 HTTPS + 动态 API）；国内客户 / 企业部署选 Nginx（运维熟悉 + 性能极致）。

---

### 6.1 Caddy 方案

#### A. 子域名路由（推荐）

每个用户一个子域名，如 `user1.api.example.com`：

```caddyfile
# /etc/caddy/Caddyfile
*.api.example.com {
    @user1 host user1.api.example.com
    @user2 host user2.api.example.com
    @user3 host user3.api.example.com

    handle @user1 {
        reverse_proxy unix//run/raisfast/user1.sock
    }
    handle @user2 {
        reverse_proxy unix//run/raisfast/user2.sock
    }
    handle @user3 {
        reverse_proxy unix//run/raisfast/user3.sock
    }
}
```

**DNS 要求**：需要通配符 DNS 记录 `*.api.example.com` 指向服务器 IP。

**Caddy 语法**：`unix//run/...` 是双斜杠，不是拼写错误。

#### B. 路径前缀路由（省域名）

所有用户共享一个域名，通过路径前缀区分：

```caddyfile
api.example.com {
    handle /user1/* {
        uri strip_prefix /user1
        reverse_proxy unix//run/raisfast/user1.sock
    }
    handle /user2/* {
        uri strip_prefix /user2
        reverse_proxy unix//run/raisfast/user2.sock
    }
    handle /user3/* {
        uri strip_prefix /user3
        reverse_proxy unix//run/raisfast/user3.sock
    }
}
```

访问路径示例：`https://api.example.com/user1/api/v1/posts`

**注意**：`uri strip_prefix` 会去掉前缀再转发给后端，raisfast 收到的还是 `/api/v1/posts`，不需要适配路径。

#### C. 动态配置（Caddy API）

实例多时手写 Caddyfile 不现实，通过 Caddy API 动态添加路由：

```bash
# 新增用户路由
curl -X POST localhost:2019/config/apps/http/servers/raisfast/routes \
  -H "Content-Type: application/json" \
  -d '{
    "@id": "user1",
    "match": [{"host": ["user1.api.example.com"]}],
    "handle": [{"handler": "reverse_proxy", "upstreams": [{"dial": "unix//run/raisfast/user1.sock"}]}]
  }'

# 删除用户路由
curl -X DELETE localhost:2019/config/apps/http/servers/raisfast/routes/user1
```

---

### 6.2 Nginx 方案

#### A. 子域名路由

```nginx
# /etc/nginx/conf.d/raisfast.conf

# 通配符证书需要手动配置（certbot 通配符需要 DNS 验证）
# certbot certonly --dns-cloudflare -d '*.api.example.com'

server {
    listen 443 ssl;
    server_name ~^(?<user>.+)\.api\.example\.com$;

    ssl_certificate     /etc/letsencrypt/live/api.example.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/api.example.com/privkey.pem;

    location / {
        proxy_pass http://unix:/run/raisfast/$user.sock;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }
}
```

**Nginx 优势**：用正则 `(?<user>.+)` 从子域名提取用户名，**一个 server 块匹配所有用户**，不需要为每个用户单独写配置。

**SSL 续签**：

```bash
# crontab -e
0 3 * * * certbot renew --quiet && systemctl reload nginx
```

#### B. 路径前缀路由

```nginx
server {
    listen 443 ssl;
    server_name api.example.com;

    ssl_certificate     /etc/letsencrypt/live/api.example.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/api.example.com/privkey.pem;

    location ~ ^/user1(/.*)$ {
        proxy_pass http://unix:/run/raisfast/user1.sock:$1;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }

    location ~ ^/user2(/.*)$ {
        proxy_pass http://unix:/run/raisfast/user2.sock:$1;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }
}
```

**注意**：Nginx 路径前缀路由需要为每个用户写一个 location 块，无法像子域名那样用正则动态匹配。实例多时建议用子域名方式。

#### C. 动态配置（脚本生成）

Nginx 没有 API，但可以用脚本生成配置后 reload：

```bash
#!/bin/bash
# nginx-add-user.sh <username>
USER=$1
cat >> /etc/nginx/conf.d/raisfast-users.conf << EOF

    location ~ ^/${USER}(/.*)$ {
        proxy_pass http://unix:/run/raisfast/${USER}.sock:\$1;
        proxy_set_header Host \$host;
        proxy_set_header X-Real-IP \$remote_addr;
        proxy_set_header X-Forwarded-For \$proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto \$scheme;
    }
EOF

nginx -t && systemctl reload nginx
```

或者用子域名方式，一个 server 块自动匹配所有用户，**完全不需要动态生成配置**。

---

## 7. 自动化脚本

### 7.1 新增用户

```bash
#!/bin/bash
# add-user.sh <username>

set -e
USER=$1
BASE=/var/lib/raisfast/$USER
SECRET=$(openssl rand -hex 32)

# 创建数据目录
mkdir -p $BASE/data

# 从模板复制空数据库
cp /var/lib/raisfast/template.db $BASE/data/raisfast.db

# 生成配置文件
cat > /etc/raisfast/users/$USER.toml << EOF
database_url = "sqlite:$BASE/data/raisfast.db"
listen = "unix:/run/raisfast/$USER.sock"
data_dir = "$BASE/data"
jwt_secret = "$SECRET"
EOF

# 设置权限
chown -R raisfast:raisfast $BASE
chmod 600 /etc/raisfast/users/$USER.toml

# 启动实例
systemctl enable --now raisfast@$USER

# 重载反向代理（二选一）
systemctl reload caddy    # Caddy
systemctl reload nginx    # Nginx

echo "用户 $USER 已创建并启动"
echo "API 地址: https://$USER.api.example.com"
```

### 7.2 删除用户

```bash
#!/bin/bash
# remove-user.sh <username>

set -e
USER=$1

systemctl stop raisfast@$USER
systemctl disable raisfast@$USER
rm /etc/raisfast/users/$USER.toml
rm -rf /var/lib/raisfast/$USER
rm -f /run/raisfast/$USER.sock

# 重载反向代理（二选一）
systemctl reload caddy    # Caddy
systemctl reload nginx    # Nginx

echo "用户 $USER 已删除"
```

### 7.3 批量状态查看

```bash
#!/bin/bash
# list-users.sh

echo "用户名       状态       PID       内存(MB)"
echo "──────────────────────────────────────────"

for config in /etc/raisfast/users/*.toml; do
    USER=$(basename "$config" .toml)
    STATUS=$(systemctl is-active "raisfast@$USER" 2>/dev/null)
    if [ "$STATUS" = "active" ]; then
        PID=$(systemctl show "raisfast@$USER" -p MainPID --value)
        MEM=$(ps -p $PID -o rss= 2>/dev/null | awk '{printf "%.0f", $1/1024}')
    else
        PID="-"
        MEM="-"
    fi
    printf "%-14s%-11s%-10s%s\n" "$USER" "$STATUS" "$PID" "$MEM"
done
```

---

## 8. 目录结构总览

```
/usr/local/bin/raisfast                          # 二进制文件
/etc/raisfast/
  users/
    user1.toml                                   # 用户配置
    user2.toml
    user3.toml
/var/lib/raisfast/
  template.db                                    # 空数据库模板
  user1/
    data/
      raisfast.db                                # 用户数据
  user2/
    data/
      raisfast.db
/run/raisfast/                                   # Unix sockets（tmpfs，重启自动清理）
  user1.sock
  user2.sock
  user3.sock
/etc/systemd/system/
  raisfast@.service                              # 模板单元
/etc/caddy/
  Caddyfile                                      # Caddy 反向代理配置
/etc/nginx/conf.d/
  raisfast.conf                                  # Nginx 反向代理配置
/etc/letsencrypt/live/                           # SSL 证书（Nginx 需要）
```

---

## 9. SQLite 调优

多实例共享磁盘 I/O，每个实例需要降低缓存占用：

```sql
-- 每个实例启动时执行
PRAGMA journal_mode = WAL;          -- 写前日志，提高并发读
PRAGMA cache_size = -8000;          -- 页缓存 8MB（默认约 2MB，单实例可给更多）
PRAGMA busy_timeout = 5000;         -- 写锁等待 5 秒
PRAGMA synchronous = NORMAL;        -- WAL 模式下安全且快速
```

**连接池**：每个实例 `max_connections = 3`（不要开太大，多实例场景下 fd 和内存都有限）。

---

## 10. 扩展路径

当单机 2GB 不够时：

| 阶段 | 方案 | 容量 |
|------|------|------|
| 初期 | 单机 2GB + SQLite 多实例 | ~50 用户 |
| 增长 | 单机 8GB + SQLite 多实例 | ~200 用户 |
| 规模化 | 切 PostgreSQL 多租户模式（所有用户共享一个进程，tenant_id 隔离） | 数千用户 |
| 大规模 | PostgreSQL + 多节点 + 负载均衡 | 无上限 |

PostgreSQL 多租户模式是规模化的关键转折点：一个进程服务所有用户，内存从 O(用户数×30MB) 降为 O(1)，但需要提前在架构层做好 `tenant_id` 隔离设计。
