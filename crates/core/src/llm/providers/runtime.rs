//! Provider 扩展专属脚本运行时（零状态 load→call→unload）。
//!
//! 执行模型照抄 [`crate::plugins::PluginManager::run_inline_script_value`]
//! 的 js 分支（plugins.rs:1070-1114，flows/agent/cron worker 三消费方）：
//! 字符串源 → load → 单函数调用 → unload，零状态残留。机制逐项转录自
//! `plugins/engine_js.rs`（借鉴矩阵见 provider-plugins.md §5.1，行级引证
//! 见各函数头注释）；各公开方法的实例块各自内联——与引擎源码
//! call_filter/call_action 的形态一致，rquickjs 生命周期下无法再抽象。
//!
//! 与应用插件宿主的边界：本运行时**只注入 `utils` 纯计算全局**
//! （unixNow/jwtSignHS256/hmacSHA256/base64/base64URL/base64URLDecode/uuid，
//! 照抄 new-api `pkg/jsplugin/utils.go:31-51`），没有任何 I/O host 函数——
//! 「纯函数契约」由构造强制。
//!
//! 传参为真实 JS 对象（照抄 new-api 插件签名形状：build `(ctx)` /
//! parse `(ctx, payload)`），不做字符串管道二次解析。

use std::sync::Arc;
use std::time::Instant;

use rquickjs::Value as Js;
use rquickjs::{AsyncContext, AsyncRuntime, Ctx, Function, Module, Object};
use serde_json::Value;

/// 运行时错误 → 桥接层映射为 `ProviderError`（§5.1 错误映射表）。
#[derive(Debug, thiserror::Error)]
pub enum RunnerError {
    /// 扩展或导出缺失（fail-fast：reload 时即暴露）。
    #[error("provider extension missing: {0}")]
    Missing(String),
    /// JS 异常（消息已提取，不含堆栈）。
    #[error("provider extension js error: {0}")]
    Js(String),
    /// 输入/返回值不可 JSON 序列化。
    #[error("provider extension serialization error: {0}")]
    Serialize(String),
    /// 执行超时（interrupt handler 触发）。
    #[error("provider extension execution timed out after {0}ms")]
    Timeout(u64),
    #[error("provider extension runtime error: {0}")]
    Internal(String),
}

type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;

/// 时钟：生产用系统时间，测试可注入固定值（照抄 new-api fixture.go:37
/// 「unixNow is fixed by the fixture」——签名断言逐字节确定）。
fn system_clock() -> Clock {
    Arc::new(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64
    })
}

#[derive(Clone)]
pub struct ProviderRunner {
    memory_limit_bytes: usize,
    now: Clock,
}

impl Default for ProviderRunner {
    fn default() -> Self {
        Self::new()
    }
}

impl ProviderRunner {
    /// 生产构造：系统时钟 + 64MB 内存上限（对齐应用插件 config 默认；
    /// 本运行时无 db/vfs 等宿主对象，64MB 绰绰有余）。
    pub fn new() -> Self {
        Self {
            memory_limit_bytes: 64 * 1024 * 1024,
            now: system_clock(),
        }
    }

    /// 测试构造：固定时钟 → 签名/时间戳断言逐字节确定。
    pub fn with_fixed_now(memory_limit_bytes: usize, now: i64) -> Self {
        Self {
            memory_limit_bytes,
            now: Arc::new(move || now),
        }
    }

    /// 评估模块，返回 `export const meta` 的 JSON 值（meta 是常量导出，
    /// 不是函数——与四个契约函数分开处理）。
    pub async fn read_meta(
        &self,
        key: &str,
        code: &str,
        timeout_ms: u64,
    ) -> Result<Value, RunnerError> {
        let start = Instant::now();
        let runtime = Self::new_instance(self.memory_limit_bytes, timeout_ms).await?;
        let ctx = Self::new_context(&runtime).await?;
        let now = self.now.clone();

        let result = ctx
            .with(|ctx| {
                inject_utils(&ctx, now.clone())
                    .map_err(|e| RunnerError::Internal(e.to_string()))?;
                let ns = Self::eval_module(&ctx, key, code)?;
                let meta: Js = ns
                    .get("meta")
                    .map_err(|e| RunnerError::Internal(e.to_string()))?;
                if meta.is_undefined() {
                    return Err(RunnerError::Missing(format!(
                        "{key}::meta (export const meta missing)"
                    )));
                }
                stringify_value(&ctx, &meta, key)
            })
            .await;

        Self::finish(runtime, ctx, start, timeout_ms, result).await
    }

    /// reload fail-fast 校验（§3.1 阶段 2）：meta 完整 + 四个契约导出存在，
    /// 单次实例内完成。
    pub async fn validate(
        &self,
        key: &str,
        code: &str,
        funcs: &[&str],
        timeout_ms: u64,
    ) -> Result<(), RunnerError> {
        let start = Instant::now();
        let runtime = Self::new_instance(self.memory_limit_bytes, timeout_ms).await?;
        let ctx = Self::new_context(&runtime).await?;
        let now = self.now.clone();

        let result = ctx
            .with(|ctx| {
                inject_utils(&ctx, now.clone())
                    .map_err(|e| RunnerError::Internal(e.to_string()))?;
                let ns = Self::eval_module(&ctx, key, code)?;
                let meta: Js = ns
                    .get("meta")
                    .map_err(|e| RunnerError::Internal(e.to_string()))?;
                if meta.is_undefined() {
                    return Err(RunnerError::Missing(format!(
                        "{key}::meta (export const meta missing)"
                    )));
                }
                stringify_value(&ctx, &meta, key)?;
                for f in funcs {
                    let _: Function = ns.get(*f).map_err(|_| {
                        RunnerError::Missing(format!("{key}::{f} (contract export missing)"))
                    })?;
                }
                Ok(())
            })
            .await;

        Self::finish(runtime, ctx, start, timeout_ms, result).await
    }

    /// 调用契约函数：build 类 `f(ctx)`，parse 类 `f(ctx, payload)`。
    pub async fn call(
        &self,
        key: &str,
        code: &str,
        func: &str,
        ctx_val: &Value,
        payload: Option<&Value>,
        timeout_ms: u64,
    ) -> Result<Value, RunnerError> {
        let start = Instant::now();
        let runtime = Self::new_instance(self.memory_limit_bytes, timeout_ms).await?;
        let ctx = Self::new_context(&runtime).await?;
        let now = self.now.clone();
        let func_name = func.to_string();

        let result = ctx
            .with(|ctx| {
                inject_utils(&ctx, now.clone())
                    .map_err(|e| RunnerError::Internal(e.to_string()))?;
                let ns = Self::eval_module(&ctx, key, code)?;
                let func: Function = ns.get(func_name.as_str()).map_err(|_| {
                    RunnerError::Missing(format!("{key}::{func_name} (contract export missing)"))
                })?;
                let ctx_js = value_to_js(&ctx, ctx_val)
                    .map_err(|e| RunnerError::Serialize(e.to_string()))?;

                let out = match payload {
                    Some(p) => {
                        let payload_js = value_to_js(&ctx, p)
                            .map_err(|e| RunnerError::Serialize(e.to_string()))?;
                        func.call((ctx_js, payload_js))
                    }
                    None => func.call((ctx_js,)),
                }
                .map_err(|e| RunnerError::Js(js_message(&ctx, &e)))?;

                stringify_value(&ctx, &out, key)
            })
            .await;

        Self::finish(runtime, ctx, start, timeout_ms, result).await
    }

    // ── 实例机制（转录自 engine_js.rs，行级引证见各 fn）─────────────

    /// engine_js.rs:181-183（内存上限/栈）+ :288-294（超时中断，同时覆盖
    /// scan 与调用全程——顶层死循环同样被斩断）。
    async fn new_instance(
        memory_limit_bytes: usize,
        timeout_ms: u64,
    ) -> Result<AsyncRuntime, RunnerError> {
        let runtime = AsyncRuntime::new().map_err(|e| RunnerError::Internal(e.to_string()))?;
        runtime.set_memory_limit(memory_limit_bytes).await;
        runtime.set_max_stack_size(512 * 1024).await;
        let start = Instant::now();
        runtime
            .set_interrupt_handler(Some(Box::new(move || {
                start.elapsed().as_millis() > u128::from(timeout_ms)
            })))
            .await;
        Ok(runtime)
    }

    async fn new_context(runtime: &AsyncRuntime) -> Result<AsyncContext, RunnerError> {
        AsyncContext::full(runtime)
            .await
            .map_err(|e| RunnerError::Internal(format!("js context: {e}")))
    }

    /// ESM 模块声明 + eval（engine_js.rs:210-212）；无 loader——import 语句
    /// 在 eval 即报错（自包含单文件，fail-fast）。
    fn eval_module<'js>(
        ctx: &Ctx<'js>,
        key: &str,
        code: &str,
    ) -> Result<rquickjs::Object<'js>, RunnerError> {
        let module = Module::declare(ctx.clone(), format!("{key}.js"), code)
            .map_err(|e| RunnerError::Js(format!("declare: {e}")))?;
        let (evaled, promise) = module
            .eval()
            .map_err(|e| RunnerError::Js(format!("eval: {}", js_message(ctx, &e))))?;
        promise
            .finish::<()>()
            .map_err(|e| RunnerError::Js(format!("eval promise: {}", js_message(ctx, &e))))?;
        evaled
            .namespace()
            .map_err(|e| RunnerError::Internal(e.to_string()))
    }

    /// 收尾：GC + 超时判定（interrupt 触发后的报错形态不定，以时钟为准）。
    async fn finish<T>(
        runtime: AsyncRuntime,
        ctx: AsyncContext,
        start: Instant,
        timeout_ms: u64,
        result: Result<T, RunnerError>,
    ) -> Result<T, RunnerError> {
        drop(ctx);
        runtime.run_gc().await;
        if start.elapsed().as_millis() > u128::from(timeout_ms) {
            return Err(RunnerError::Timeout(timeout_ms));
        }
        result
    }
}

/// 返回值/常量导出 → serde Value：undefined/不可序列化 → Err。
/// 转录自 engine_js.rs:327-331。
fn stringify_value<'js>(ctx: &Ctx<'js>, v: &Js<'js>, key: &str) -> Result<Value, RunnerError> {
    let serialized = ctx
        .json_stringify(v.clone())
        .map_err(|e| RunnerError::Serialize(e.to_string()))?
        .ok_or_else(|| RunnerError::Serialize(format!("{key}: value is undefined")))?;
    let text = serialized
        .to_string()
        .map_err(|e| RunnerError::Serialize(e.to_string()))?;
    serde_json::from_str(text.as_str()).map_err(|e| RunnerError::Serialize(e.to_string()))
}

/// 提取 JS 异常的真实消息（替代 "Exception generated by QuickJS" 包装）。
/// 转录自 engine_js.rs:314-323。
fn js_message(ctx: &Ctx<'_>, e: &rquickjs::Error) -> String {
    let caught = ctx.catch();
    caught
        .as_string()
        .and_then(|s| s.to_string().ok())
        .or_else(|| {
            caught
                .as_object()
                .and_then(|o| o.get::<&str, String>("message").ok())
        })
        .unwrap_or_else(|| e.to_string())
}

/// serde_json::Value → rquickjs Value（递归；JSON 数字以 f64 承载，
/// i32 范围内保持整型便于 JS 侧相等比较）。
fn value_to_js<'js>(ctx: &Ctx<'js>, v: &Value) -> rquickjs::Result<Js<'js>> {
    Ok(match v {
        Value::Null => Js::new_null(ctx.clone()),
        Value::Bool(b) => Js::new_bool(ctx.clone(), *b),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                if (i32::MIN as i64..=i32::MAX as i64).contains(&i) {
                    Js::new_int(ctx.clone(), i as i32)
                } else {
                    Js::new_float(ctx.clone(), i as f64)
                }
            } else {
                Js::new_float(ctx.clone(), n.as_f64().unwrap_or_default())
            }
        }
        Value::String(s) => rquickjs::String::from_str(ctx.clone(), s.as_str())?.into(),
        Value::Array(items) => {
            let arr = rquickjs::Array::new(ctx.clone())?;
            for (i, item) in items.iter().enumerate() {
                arr.set(i, value_to_js(ctx, item)?)?;
            }
            arr.into_value()
        }
        Value::Object(map) => {
            let obj = Object::new(ctx.clone())?;
            for (k, item) in map {
                obj.set(k.as_str(), value_to_js(ctx, item)?)?;
            }
            obj.into_value()
        }
    })
}

// ── utils 纯计算全局（§4.4，清单照抄 new-api utils.go:31-51）────────
//
// 全部为 string→string 的纯计算闭包（FromJs 零摩擦）；jwtSignHS256 的
// 组装在 JS 侧 prelude 完成（header/payload base64url + Rust HMAC 签名），
// node 测试可整体 stub。

fn inject_utils(ctx: &Ctx<'_>, now: Clock) -> rquickjs::Result<()> {
    let global = ctx.globals();
    let utils = Object::new(ctx.clone())?;

    let now_unix = now.clone();
    utils.set("unixNow", Function::new(ctx.clone(), move || (now_unix)()))?;
    utils.set(
        "hmacSHA256",
        Function::new(
            ctx.clone(),
            |message: String, secret: String| -> rquickjs::Result<String> {
                hmac_sha256_hex(&message, &secret)
            },
        ),
    )?;
    utils.set(
        "base64",
        Function::new(ctx.clone(), |value: String| -> String {
            use base64::Engine as _;
            base64::engine::general_purpose::STANDARD.encode(value.as_bytes())
        }),
    )?;
    utils.set(
        "base64URL",
        Function::new(ctx.clone(), |value: String| -> String {
            use base64::Engine as _;
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(value.as_bytes())
        }),
    )?;
    utils.set(
        "base64URLDecode",
        Function::new(ctx.clone(), |value: String| -> rquickjs::Result<String> {
            use base64::Engine as _;
            let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(value.as_bytes())
                .map_err(|e| rquickjs::Error::FromJs {
                    from: "string",
                    to: "base64URL-decoded string",
                    message: Some(e.to_string()),
                })?;
            Ok(String::from_utf8_lossy(&decoded).into_owned())
        }),
    )?;
    utils.set(
        "uuid",
        Function::new(ctx.clone(), || uuid::Uuid::now_v7().to_string()),
    )?;

    global.set("utils", utils)?;
    // jwtSignHS256 组装逻辑以 prelude 绑定（utils 对象已就位）。
    ctx.eval::<(), _>(PRELUDE_JWT_BIND)
}

const PRELUDE_JWT_BIND: &str = r#"
utils.jwtSignHS256 = (claims, secret) => {
    if (!claims || typeof claims !== 'object') throw new TypeError('jwtSignHS256: claims must be an object');
    if (typeof secret !== 'string' || secret.length === 0) throw new TypeError('jwtSignHS256: secret must be a non-empty string');
    const header = utils.base64URL('{"alg":"HS256","typ":"JWT"}');
    const payload = utils.base64URL(JSON.stringify(claims));
    const signingInput = header + '.' + payload;
    return signingInput + '.' + utils.hmacSHA256(signingInput, secret);
};"#;

fn hmac_sha256_hex(message: &str, secret: &str) -> Result<String, rquickjs::Error> {
    use hmac::{Hmac, KeyInit, Mac};
    type HmacSha256 = Hmac<sha2::Sha256>;
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).map_err(|e| rquickjs::Error::FromJs {
            from: "string",
            to: "hmac key",
            message: Some(e.to_string()),
        })?;
    mac.update(message.as_bytes());
    let out = mac.finalize().into_bytes();
    Ok(hex::encode(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXED_NOW: i64 = 1_700_000_000;

    fn runner() -> ProviderRunner {
        ProviderRunner::with_fixed_now(16 * 1024 * 1024, FIXED_NOW)
    }

    const META_SRC: &str = r#"
export const meta = { key: "demo", version: "1.0.0", contract: 1, protocols: ["video"], http: ["demo.example/*"], timeout_ms: 30000 };
export function echoCtx(ctx) { return ctx; }
"#;

    #[tokio::test]
    async fn read_meta_parses_const_export() {
        let meta = runner().read_meta("demo", META_SRC, 5000).await.unwrap();
        assert_eq!(meta["key"], "demo");
        assert_eq!(meta["contract"], 1);
        assert_eq!(meta["http"][0], "demo.example/*");
    }

    #[tokio::test]
    async fn validate_missing_export_is_hard_error() {
        let err = runner()
            .validate(
                "demo",
                META_SRC,
                &[
                    "echoCtx",
                    "buildSubmitRequest",
                    "parseSubmitResponse",
                    "buildQueryRequest",
                    "parseTaskResult",
                ],
                5000,
            )
            .await
            .unwrap_err();
        assert!(
            matches!(err, RunnerError::Missing(ref m) if m.contains("buildSubmitRequest")),
            "{err}"
        );
    }

    #[tokio::test]
    async fn validate_passes_full_contract() {
        let src = format!(
            "{META_SRC}\nexport function buildSubmitRequest(ctx) {{ return ctx; }}\nexport function parseSubmitResponse(ctx, r) {{ return r; }}\nexport function buildQueryRequest(ctx) {{ return ctx; }}\nexport function parseTaskResult(ctx, r) {{ return r; }}\n"
        );
        runner()
            .validate(
                "demo",
                &src,
                &[
                    "echoCtx",
                    "buildSubmitRequest",
                    "parseSubmitResponse",
                    "buildQueryRequest",
                    "parseTaskResult",
                ],
                5000,
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn missing_meta_fails_fast() {
        let err = runner()
            .read_meta("demo", "export function f(){ return 1; }", 5000)
            .await
            .unwrap_err();
        assert!(
            matches!(err, RunnerError::Missing(ref m) if m.contains("meta")),
            "{err}"
        );
    }

    #[tokio::test]
    async fn call_passes_ctx_as_real_object() {
        let out = runner()
            .call(
                "demo",
                META_SRC,
                "echoCtx",
                &serde_json::json!({"a": 1, "nested": {"s": "x"}, "arr": [true, null]}),
                None,
                5000,
            )
            .await
            .unwrap();
        assert_eq!(out["a"], 1);
        assert_eq!(out["nested"]["s"], "x");
        assert_eq!(out["arr"][0], true);
    }

    #[tokio::test]
    async fn parse_variant_receives_payload_as_second_arg() {
        let src = "export function pick(ctx, payload) { return { fromCtx: ctx.k, fromPayload: payload.v }; }";
        let out = runner()
            .call(
                "demo",
                src,
                "pick",
                &serde_json::json!({"k": 7}),
                Some(&serde_json::json!({"v": "upstream"})),
                5000,
            )
            .await
            .unwrap();
        assert_eq!(out["fromCtx"], 7);
        assert_eq!(out["fromPayload"], "upstream");
    }

    #[tokio::test]
    async fn undefined_return_is_serialize_error() {
        let src = "export function nothing(ctx) { return undefined; }";
        let err = runner()
            .call("demo", src, "nothing", &serde_json::json!({}), None, 5000)
            .await
            .unwrap_err();
        assert!(
            matches!(err, RunnerError::Serialize(ref m) if m.contains("undefined")),
            "{err}"
        );
    }

    #[tokio::test]
    async fn js_throw_surfaces_real_message() {
        let src = "export function boom(ctx) { throw new Error('upstream says no'); }";
        let err = runner()
            .call("demo", src, "boom", &serde_json::json!({}), None, 5000)
            .await
            .unwrap_err();
        assert!(
            matches!(err, RunnerError::Js(ref m) if m.contains("upstream says no")),
            "{err}"
        );
    }

    #[tokio::test]
    async fn top_level_infinite_loop_hits_timeout() {
        let src = "export const meta = {key:'d',contract:1}; export function spin(ctx) { while (true) {} }";
        let err = runner()
            .call("d", src, "spin", &serde_json::json!({}), None, 100)
            .await
            .unwrap_err();
        assert!(matches!(err, RunnerError::Timeout(100)), "{err}");
    }

    #[tokio::test]
    async fn import_statement_fails_fast_at_eval() {
        let src = "import fs from 'fs';\nexport function f(ctx) { return 1; }";
        let err = runner()
            .call("demo", src, "f", &serde_json::json!({}), None, 5000)
            .await
            .unwrap_err();
        assert!(!matches!(err, RunnerError::Missing(_)), "{err}");
    }

    #[tokio::test]
    async fn utils_unix_now_uses_injected_clock() {
        let src = "export function when(ctx) { return utils.unixNow(); }";
        let out = runner()
            .call("demo", src, "when", &serde_json::json!({}), None, 5000)
            .await
            .unwrap();
        assert_eq!(out, Value::from(FIXED_NOW));
    }

    /// RFC 4231 test case 2 的已知向量。
    #[tokio::test]
    async fn utils_hmac_sha256_known_vector() {
        let src = r#"export function sign(ctx) { return utils.hmacSHA256("what do ya want for nothing?", "Jefe"); }"#;
        let out = runner()
            .call("demo", src, "sign", &serde_json::json!({}), None, 5000)
            .await
            .unwrap();
        assert_eq!(
            out,
            Value::from("5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843")
        );
    }

    #[tokio::test]
    async fn utils_jwt_sign_hs256_assembles_kling_style_token() {
        let src = r#"
export function token(ctx) {
    return utils.jwtSignHS256({ iss: ctx.iss, exp: ctx.exp, nbf: ctx.nbf }, ctx.secret);
}"#;
        let out = runner()
            .call(
                "demo",
                src,
                "token",
                &serde_json::json!({"iss": "ak-test", "exp": FIXED_NOW + 1800, "nbf": FIXED_NOW - 5, "secret": "sk-test"}),
                None,
                5000,
            )
            .await
            .unwrap();
        let token = out.as_str().unwrap();
        let parts: Vec<&str> = token.split('.').collect();
        assert_eq!(parts.len(), 3, "{token}");
        assert_eq!(
            parts[0], "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9",
            "HS256 header"
        );
        // 签名逐字节验证：同一条 signing input 的 HMAC 必须一致。
        let expected = {
            use hmac::{Hmac, KeyInit, Mac};
            type HmacSha256 = Hmac<sha2::Sha256>;
            let mut mac = HmacSha256::new_from_slice(b"sk-test").unwrap();
            mac.update(format!("{}.{}", parts[0], parts[1]).as_bytes());
            hex::encode(mac.finalize().into_bytes())
        };
        assert_eq!(parts[2], expected);
        let payload = {
            use base64::Engine as _;
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(parts[1])
                .unwrap()
        };
        let claims: Value = serde_json::from_slice(&payload).unwrap();
        assert_eq!(claims["iss"], "ak-test");
        assert_eq!(claims["exp"], FIXED_NOW + 1800);
    }
}
