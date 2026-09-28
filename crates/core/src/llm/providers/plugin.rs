//! PluginProvider：provider 扩展 → `ModelProvider` video 面的桥接层。
//!
//! 职责（provider-plugins.md §5.2）：
//! - 组装 ctx（apiKey/baseUrl/model/paramOverride/request/taskId，§4.3）；
//! - `runner.call("buildSubmitRequest", ctx)` → spec 校验（url/method/白名单）
//!   → host 代发（status 透传，信封分类交扩展）→
//!   `runner.call("parseSubmitResponse", ctx, resp)`；
//! - video_content：host 直接下载 parseTaskResult 给出的 url（不过 JS、
//!   不走扩展白名单——成片 CDN 域与 API 域不同是常态）；
//! - 错误映射：JS 异常/缺失/序列化 → Config；HTTP → Http（status 透传）；
//!   网络/超时 → Transport；
//! - 状态映射：未知 → InProgress + warn（禁用 `VideoStatus::from_wire`，
//!   与原生 wan/vidu/seedance 的 sweep 容忍语义对齐）。
//!
//! 参考：构造面照抄 `providers/kling.rs`；下载与 redact 照抄
//! kling/seedance；健康记账照抄 plugins.rs record_hook_error 模式。

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Value, json};

use raisfast_agent::provider::{
    ChatRequest, ChatResponse, ModelProvider, ProviderError, VideoRequest, VideoStatus, VideoTask,
};

use super::ext::{ProviderExt, ProviderExtRegistry};
use super::runtime::RunnerError;
use crate::llm::relay::adaptor::shared_client;
use crate::plugins::Permissions;
use crate::plugins::permissions::PermissionChecker;

pub struct PluginProvider {
    http: reqwest::Client,
    registry: Arc<ProviderExtRegistry>,
    key: String,
    display_name: String,
    base_url: String,
    api_key: Option<String>,
    param_override: Option<Value>,
    header_override: Option<Value>,
}

impl PluginProvider {
    pub fn new(
        registry: Arc<ProviderExtRegistry>,
        key: impl Into<String>,
        base_url: impl Into<String>,
        api_key: Option<String>,
        param_override: Option<Value>,
        header_override: Option<Value>,
    ) -> Self {
        let key = key.into();
        Self {
            http: shared_client().clone(),
            registry,
            display_name: format!("ext:{key}"),
            key,
            base_url: base_url.into(),
            api_key,
            param_override,
            header_override,
        }
    }

    /// 扩展可用性（未加载 / 自动禁用 → Config，内核按渠道级失败处理）。
    fn ext_available(&self) -> Result<Arc<ProviderExt>, ProviderError> {
        let ext = self.registry.get(&self.key).ok_or_else(|| {
            ProviderError::Config(format!("provider extension {} not loaded", self.key))
        })?;
        if !ext.is_available() {
            return Err(ProviderError::Config(format!(
                "provider extension {} auto-disabled after consecutive errors",
                self.key
            )));
        }
        Ok(ext)
    }

    /// ctx 组装（§4.3）。apiKey 缺失注入空串——无 key 渠道在上游鉴权处
    /// 失败，桥接不替渠道做策略。
    fn build_ctx(
        &self,
        model: &str,
        request: Option<&VideoRequest>,
        task_id: Option<&str>,
        task_data: Option<&Value>,
    ) -> Value {
        let request_json = request.map(|r| {
            json!({
                "prompt": r.prompt,
                "seconds": r.seconds,
                "size": r.size,
                "inputReferences": r.input_references.iter().map(|ref_| json!({
                    "url": ref_.url,
                    "b64Json": ref_.b64_json,
                    "mime": ref_.mime,
                })).collect::<Vec<_>>(),
                "callbackUrl": r.callback_url,
            })
        });
        json!({
            "provider": self.key,
            "baseUrl": self.base_url,
            "apiKey": self.api_key.clone().unwrap_or_default(),
            "model": model,
            "paramOverride": self.param_override.clone().unwrap_or(Value::Null),
            "request": request_json,
            "taskId": task_id.map(str::to_string),
            // 提交期状态回传（kling 双路径查询用 action 选端点，§4.3）。
            "taskData": task_data.cloned().unwrap_or(Value::Null),
        })
    }

    /// runner 调用 + 健康记账 + 错误映射（§5.1 错误映射表）。
    async fn runner_call(
        &self,
        ext: &ProviderExt,
        func: &str,
        ctx: &Value,
        payload: Option<&Value>,
    ) -> Result<Value, ProviderError> {
        let result = self
            .registry
            .runner()
            .call(&self.key, &ext.code, func, ctx, payload, ext.timeout_ms)
            .await;
        match result {
            Ok(v) => {
                self.registry.record_result(&self.key, true);
                Ok(v)
            }
            Err(RunnerError::Timeout(ms)) => {
                let disabled = self.registry.record_result(&self.key, false);
                if disabled {
                    tracing::warn!(provider = %self.key, "provider extension auto-disabled after consecutive errors");
                }
                Err(ProviderError::Transport(self.redact(&format!(
                    "provider extension {} timed out after {ms}ms",
                    self.key
                ))))
            }
            Err(e @ (RunnerError::Missing(_) | RunnerError::Js(_) | RunnerError::Serialize(_))) => {
                let disabled = self.registry.record_result(&self.key, false);
                if disabled {
                    tracing::warn!(provider = %self.key, "provider extension auto-disabled after consecutive errors");
                }
                Err(ProviderError::Config(
                    self.redact(&format!("provider extension {}: {e}", self.key)),
                ))
            }
            Err(e @ RunnerError::Internal(_)) => Err(ProviderError::Transport(
                self.redact(&format!("provider extension {}: {e}", self.key)),
            )),
        }
    }

    /// host 代发（§5.3）：GET/POST only、白名单逐请求校验、status 透传、
    /// 超时随扩展 meta。日志纪律：headers 永不落日志（key 可能在其中）。
    async fn send_spec(&self, ext: &ProviderExt, spec: &Value) -> Result<Value, ProviderError> {
        let url = spec.get("url").and_then(Value::as_str).ok_or_else(|| {
            ProviderError::Config(format!(
                "provider extension {}: spec.url is required",
                self.key
            ))
        })?;
        let method = spec.get("method").and_then(Value::as_str).unwrap_or("POST");
        if method != "GET" && method != "POST" {
            return Err(ProviderError::Config(format!(
                "provider extension {}: only GET/POST methods are allowed, got {method}",
                self.key
            )));
        }
        let permissions = Permissions {
            http: ext.http.clone(),
            ..Default::default()
        };
        if !PermissionChecker::is_url_allowed(&permissions, url) {
            return Err(ProviderError::Config(format!(
                "provider extension {}: url denied by http allowlist: {url}",
                self.key
            )));
        }

        let mut req = match method {
            "GET" => self.http.get(url),
            _ => self.http.post(url),
        };
        if let Some(headers) = spec.get("headers").and_then(Value::as_object) {
            for (name, value) in headers {
                if let Some(vs) = value.as_str() {
                    req = req.header(name.as_str(), vs);
                }
            }
        }
        if let Some(over) = &self.header_override
            && let Some(map) = over.as_object()
        {
            for (name, value) in map {
                if let Some(vs) = value.as_str() {
                    req = req.header(name.as_str(), vs);
                }
            }
        }
        if let Some(body) = spec.get("body")
            && body.is_object()
        {
            req = req.json(body);
        }
        let resp = req
            .timeout(Duration::from_millis(ext.timeout_ms))
            .send()
            .await
            .map_err(|e| ProviderError::Transport(e.to_string()))?;
        let status = resp.status().as_u16();
        let text = resp
            .text()
            .await
            .map_err(|e| ProviderError::Transport(e.to_string()))?;
        let body = serde_json::from_str::<Value>(&text).unwrap_or(Value::String(text));
        Ok(json!({ "status": status, "body": body }))
    }

    /// 状态映射：未知 → InProgress + warn（禁用 `VideoStatus::from_wire`——
    /// 它把未知映射为 Failed，对 sweep 不容忍）。
    fn status_from_wire(key: &str, state: &str) -> VideoStatus {
        match state {
            "queued" => VideoStatus::Queued,
            "in_progress" => VideoStatus::InProgress,
            "completed" => VideoStatus::Completed,
            "failed" => VideoStatus::Failed,
            other => {
                tracing::warn!(
                    provider = %key,
                    state = %other,
                    "provider extension: unknown task status, treating as in-progress"
                );
                VideoStatus::InProgress
            }
        }
    }

    /// 轮询 + 解析；同时返回 parseTaskResult 给出的成片 url（content 用）。
    /// `task_data`：提交期状态回传（kling action 等，调用方持久化）。
    /// 非 2xx → `ProviderError::Http`（parse 只见 2xx）。
    async fn query_full(
        &self,
        task_id: &str,
        model: &str,
        task_data: Option<&Value>,
    ) -> Result<(VideoTask, Option<String>), ProviderError> {
        let ext = self.ext_available()?;
        let ctx = self.build_ctx(model, None, Some(task_id), task_data);
        let spec = self
            .runner_call(&ext, "buildQueryRequest", &ctx, None)
            .await?;
        let resp = self.send_spec(&ext, &spec).await?;
        if let Some(e) = self.non_2xx_error(&resp) {
            return Err(e);
        }
        let out = self
            .runner_call(&ext, "parseTaskResult", &ctx, Some(&resp))
            .await?;

        let status_str = out.get("status").and_then(Value::as_str).unwrap_or("");
        let status = Self::status_from_wire(self.key.as_str(), status_str);
        let progress = out
            .get("progress")
            .and_then(Value::as_i64)
            .map(|p| p.clamp(0, 100) as i32);
        let error = out
            .get("error")
            .and_then(Value::as_str)
            .map(str::to_string)
            .filter(|_| status == VideoStatus::Failed);
        let url = out
            .get("url")
            .and_then(Value::as_str)
            .map(str::to_string)
            .filter(|_| status == VideoStatus::Completed);
        Ok((
            VideoTask {
                id: task_id.to_string(),
                status,
                progress,
                error,
                data: None,
            },
            url,
        ))
    }

    /// 非 2xx → `ProviderError::Http`（parse 只处理 2xx 信封——HTTP 层
    /// 失败与协议层失败的分类边界，与原生 provider 的 unwrap_envelope 一致）。
    fn non_2xx_error(&self, resp: &Value) -> Option<ProviderError> {
        let status = resp.get("status").and_then(Value::as_i64)?;
        if (200..300).contains(&status) {
            return None;
        }
        let body = resp.get("body").map(|b| b.to_string()).unwrap_or_default();
        Some(ProviderError::Http {
            status: status as u16,
            body: self.redact(&body),
        })
    }

    /// 错误体脱敏 [照抄 seedance.rs:163 redact]：渠道 key 不进日志/错误。
    fn redact(&self, text: &str) -> String {
        let mut out = text.to_string();
        if let Some(key) = &self.api_key
            && !key.is_empty()
        {
            out = out.replace(key.as_str(), "***");
        }
        out.chars().take(500).collect()
    }
}

#[async_trait]
impl ModelProvider for PluginProvider {
    fn name(&self) -> &str {
        &self.display_name
    }

    /// 扩展无 chat 面 → Config，内核跳过该渠道的其他模态。
    async fn chat(
        &self,
        _request: &ChatRequest<'_>,
        _model: &str,
    ) -> Result<ChatResponse, ProviderError> {
        Err(ProviderError::Config(format!(
            "provider extension {} does not support chat",
            self.key
        )))
    }

    /// 提交：buildSubmitRequest → host 代发 → parseSubmitResponse。
    async fn video_submit(
        &self,
        request: &VideoRequest,
        model: &str,
    ) -> Result<VideoTask, ProviderError> {
        let ext = self.ext_available()?;

        // host 侧通用 seconds 上限（§5.4，仅扩展路径——原生 provider 有
        // 自己的付费档位硬校验）。
        if let Some(seconds) = &request.seconds {
            let n: i64 = seconds.trim().parse().map_err(|_| {
                ProviderError::Config(format!(
                    "provider extension {}: seconds must be an integer, got {seconds:?}",
                    self.key
                ))
            })?;
            let max = self.registry.max_seconds();
            if !(1..=max).contains(&n) {
                return Err(ProviderError::Config(format!(
                    "provider extension {}: seconds must be between 1 and {max}, got {n}",
                    self.key
                )));
            }
        }

        let ctx = self.build_ctx(model, Some(request), None, None);
        let spec = self
            .runner_call(&ext, "buildSubmitRequest", &ctx, None)
            .await?;
        let resp = self.send_spec(&ext, &spec).await?;
        if let Some(e) = self.non_2xx_error(&resp) {
            return Err(e);
        }
        let out = self
            .runner_call(&ext, "parseSubmitResponse", &ctx, Some(&resp))
            .await?;

        let task_id = out
            .get("taskId")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if task_id.is_empty() {
            return Err(ProviderError::Parse(format!(
                "provider extension {}: parseSubmitResponse returned no taskId",
                self.key
            )));
        }
        let status = out
            .get("status")
            .and_then(Value::as_str)
            .map(|s| Self::status_from_wire(self.key.as_str(), s))
            .unwrap_or(VideoStatus::Queued);
        Ok(VideoTask {
            id: task_id,
            status,
            progress: None,
            error: out.get("error").and_then(Value::as_str).map(str::to_string),
            data: out.get("taskData").cloned().filter(|v| !v.is_null()),
        })
    }

    /// 轮询：buildQueryRequest → host GET → parseTaskResult。
    async fn video_query(
        &self,
        task_id: &str,
        model: &str,
        task_data: Option<&Value>,
    ) -> Result<VideoTask, ProviderError> {
        let (task, _url) = self.query_full(task_id, model, task_data).await?;
        Ok(task)
    }

    /// 成片：host 直接下载 parseTaskResult 给出的 url（不过 JS）。
    async fn video_content(
        &self,
        task_id: &str,
        model: &str,
        task_data: Option<&Value>,
    ) -> Result<Vec<u8>, ProviderError> {
        let (_task, url) = self.query_full(task_id, model, task_data).await?;
        let Some(url) = url else {
            return Err(ProviderError::Parse(format!(
                "provider extension {}: task {task_id} has no result video url",
                self.key
            )));
        };
        let resp = self
            .http
            .get(&url)
            .timeout(Duration::from_secs(300))
            .send()
            .await
            .map_err(|e| ProviderError::Transport(e.to_string()))?;
        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            return Err(ProviderError::Http {
                status,
                body: format!("provider extension {}: download failed {status}", self.key),
            });
        }
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| ProviderError::Transport(e.to_string()))?;
        Ok(bytes.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisfast_agent::provider::VideoInputRef;

    /// demo 扩展：build 会产出指向 meta.http 白名单外/内的 URL（由用例决定），
    /// parse 恒抛异常（用于驱动健康记账，不经网络）。
    fn demo_src(allow_host: &str) -> String {
        format!(
            r#"
export const meta = {{ key: "demo", name: "Demo", version: "0.1.0", contract: 1, protocols: ["video"], http: ["{allow_host}/*"], timeout_ms: 30000 }};
export function buildSubmitRequest(ctx) {{
    return {{ url: ctx.baseUrl + "/submit", method: "POST",
        headers: {{ Authorization: "Token " + ctx.apiKey }},
        body: {{ model: ctx.model, prompt: ctx.request.prompt }} }};
}}
export function parseSubmitResponse(ctx, response) {{
    throw new Error("parse should be unreachable in these tests");
}}
export function buildQueryRequest(ctx) {{
    return {{ url: ctx.baseUrl + "/poll/" + ctx.taskId, method: "GET" }};
}}
export function parseTaskResult(ctx, response) {{
    const s = response.body.status;
    const out = {{ status: s }};
    if (s === "completed") out.url = response.body.url;
    return out;
}}
"#
        )
    }

    async fn registry_with(allow_host: &str) -> Arc<ProviderExtRegistry> {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("demo.js"), demo_src(allow_host)).unwrap();
        let reg = Arc::new(ProviderExtRegistry::new(vec![]));
        reg.configure_dir(tmp.path());
        reg.reload().await.unwrap();
        // registry 只持有 code 字符串；tmp 生命周期由 mem::forget 延长
        std::mem::forget(tmp);
        reg
    }

    fn provider(reg: Arc<ProviderExtRegistry>, base_url: &str) -> PluginProvider {
        PluginProvider::new(
            reg,
            "demo",
            base_url.to_string(),
            Some("demo-secret".to_string()),
            None,
            None,
        )
    }

    fn video_request(prompt: &str, seconds: Option<&str>) -> VideoRequest {
        VideoRequest {
            prompt: prompt.into(),
            seconds: seconds.map(str::to_string),
            size: None,
            input_references: Vec::new(),
            callback_url: None,
        }
    }

    #[tokio::test]
    async fn seconds_cap_blocks_extension_path() {
        let reg = registry_with("upstream.example").await;
        let p = provider(reg, "https://upstream.example");
        let err = p
            .video_submit(&video_request("x", Some("700")), "m")
            .await
            .unwrap_err();
        assert!(
            matches!(err, ProviderError::Config(ref m) if m.contains("seconds must be between 1 and 600")),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn missing_extension_key_is_config_error() {
        let reg = Arc::new(ProviderExtRegistry::new(vec![]));
        let p = PluginProvider::new(reg, "nope", "https://upstream.example", None, None, None);
        let err = p
            .video_submit(&video_request("x", None), "m")
            .await
            .unwrap_err();
        assert!(
            matches!(err, ProviderError::Config(ref m) if m.contains("not loaded")),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn allowlist_denies_url_outside_meta_patterns() {
        // baseUrl 与白名单同 host，但 path 白名单是 /* —— host 匹配 → 放行；
        // 这里反着验证：白名单允许 upstream.example，url 恰好同 host → 放行，
        // 请求会因网络不可达（.invalid TLD）失败 → Transport。
        let reg = registry_with("upstream.example").await;
        let p = provider(reg, "https://upstream.example");
        let err = p
            .video_submit(&video_request("x", Some("5")), "m")
            .await
            .unwrap_err();
        assert!(
            matches!(err, ProviderError::Transport(_)),
            "allowlist 放行后应走到网络层: {err:?}"
        );
    }

    #[tokio::test]
    async fn allowlist_denies_foreign_host_and_private_host() {
        // 白名单只允许 upstream.example；baseUrl 换成别的公网域 → allowlist 拒绝
        let p = provider(
            registry_with("upstream.example").await,
            "https://other.example",
        );
        let err = p
            .video_submit(&video_request("x", Some("5")), "m")
            .await
            .unwrap_err();
        assert!(
            matches!(err, ProviderError::Config(ref m) if m.contains("allowlist")),
            "{err:?}"
        );

        // 私网 host（127.0.0.1）被 is_private_host 硬拒（SSRF 守卫）
        let p2 = provider(registry_with("127.0.0.1").await, "http://127.0.0.1:1");
        let err = p2
            .video_submit(&video_request("x", Some("5")), "m")
            .await
            .unwrap_err();
        assert!(
            matches!(err, ProviderError::Config(ref m) if m.contains("allowlist")),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn consecutive_runner_errors_auto_disable_extension() {
        // buildSubmitRequest 本身抛异常（不触网）→ Config；连续 3 次自动禁用
        // → 第 4 次调用在 ext_available 即报 auto-disabled。
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("throwing.js"),
            r#"
export const meta = { key: "demo", name: "Throwing", version: "0.1.0", contract: 1, protocols: ["video"], http: ["upstream.example/*"], timeout_ms: 30000 };
export function buildSubmitRequest(ctx) { throw new Error("boom-from-extension"); }
export function parseSubmitResponse(ctx, r) { return { taskId: "x" }; }
export function buildQueryRequest(ctx) { return { url: ctx.baseUrl + "/p" }; }
export function parseTaskResult(ctx, r) { return { status: "in_progress" }; }
"#,
        )
        .unwrap();
        let reg = Arc::new(ProviderExtRegistry::new(vec![]));
        reg.configure_dir(tmp.path());
        reg.reload().await.unwrap();
        std::mem::forget(tmp);

        let p = provider(reg, "https://upstream.example");
        for _ in 0..3 {
            let err = p
                .video_submit(&video_request("x", Some("5")), "m")
                .await
                .unwrap_err();
            assert!(
                matches!(err, ProviderError::Config(ref m) if m.contains("boom-from-extension")),
                "{err:?}"
            );
        }
        let err = p
            .video_submit(&video_request("x", Some("5")), "m")
            .await
            .unwrap_err();
        assert!(
            matches!(err, ProviderError::Config(ref m) if m.contains("auto-disabled")),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn status_mapping_unknown_state_tolerates_as_in_progress() {
        // 纯函数断言：未知状态 → InProgress（与原生 sweep 容忍语义一致）
        assert_eq!(
            PluginProvider::status_from_wire("demo", "queued"),
            VideoStatus::Queued
        );
        assert_eq!(
            PluginProvider::status_from_wire("demo", "in_progress"),
            VideoStatus::InProgress
        );
        assert_eq!(
            PluginProvider::status_from_wire("demo", "completed"),
            VideoStatus::Completed
        );
        assert_eq!(
            PluginProvider::status_from_wire("demo", "failed"),
            VideoStatus::Failed
        );
        assert_eq!(
            PluginProvider::status_from_wire("demo", "brand-new-state"),
            VideoStatus::InProgress
        );
    }

    #[tokio::test]
    async fn video_request_references_survive_ctx_roundtrip() {
        // ctx.request.inputReferences 形状（§4.5）经真实 JS 往返保持
        let reg = registry_with("upstream.example").await;
        let ext = reg.get("demo").unwrap();
        let p = provider(reg.clone(), "https://upstream.example");
        let request = VideoRequest {
            prompt: "p".into(),
            seconds: Some("5".into()),
            size: Some("1280x720".into()),
            input_references: vec![VideoInputRef::from_url("https://img.example/a.png")],
            callback_url: None,
        };
        let ctx = p.build_ctx("m", Some(&request), None, None);
        assert_eq!(ctx["request"]["size"], "1280x720");
        assert_eq!(
            ctx["request"]["inputReferences"][0]["url"],
            "https://img.example/a.png"
        );
        let _ = ext;
    }
}
