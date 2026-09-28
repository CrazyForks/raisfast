//! Provider 扩展注册表：`extensions/llm_providers/*.js` 的扫描、校验、
//! 健康（连续错误自动禁用）与热 reload。
//!
//! 扫描/逐文件加载/失败继续照抄 `plugins.rs:468-545` load_all 模式；
//! 健康阈值自动禁用照抄 `plugins.rs:1818` record_hook_error 模式——均为
//! 独立实现，不经 PluginManager（两套扩展零共享，见 provider-plugins.md
//! §3.1 阶段 2）。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc as StdArc;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, Ordering};

use serde_json::Value;

use super::runtime::{ProviderRunner, RunnerError};

/// 当前内核支持的扩展契约版本（meta.contract 不匹配 → 拒绝注册）。
pub const SUPPORTED_CONTRACT: u32 = 1;
/// 契约要求的四个导出函数（§4.2，video 模态）。
pub const CONTRACT_FUNCS: [&str; 4] = [
    "buildSubmitRequest",
    "parseSubmitResponse",
    "buildQueryRequest",
    "parseTaskResult",
];
/// 支持的协议清单及各协议要求的导出函数（多模态扩展，按声明校验）。
pub const PROTOCOL_FUNCS: &[(&str, &[&str])] = &[
    ("video", &CONTRACT_FUNCS),
    ("chat", &["buildChatRequest", "parseChatResponse"]),
    ("speech", &["buildSpeechRequest", "parseSpeechResponse"]),
    ("music", &["buildMusicRequest", "parseMusicResponse"]),
    ("image", &["buildImageRequest", "parseImageResponse"]),
];
/// 连续错误自动禁用阈值 [照抄本仓 PluginManager AUTO_DISABLE_THRESHOLD 模式]。
const AUTO_DISABLE_THRESHOLD: u32 = 3;
/// meta.timeout_ms 缺省（视频 submit 上游常 >5s，勿取小值）。
pub const DEFAULT_TIMEOUT_MS: u64 = 30_000;

/// 一个已注册的 provider 扩展（只读快照 + 健康计数）。
pub struct ProviderExt {
    pub key: String,
    pub name: String,
    pub version: String,
    pub contract: u32,
    pub protocols: Vec<String>,
    /// 出网白名单 pattern（host 代发前逐请求校验，§6）。
    pub http: Vec<String>,
    pub timeout_ms: u64,
    /// 建议模型清单（UX 展示/预填；权威源仍是 llm_models 目录，非定价依据）。
    pub models: Vec<String>,
    /// 展示描述（管理台；UX-only）。
    pub description: String,
    pub file: PathBuf,
    pub code: StdArc<String>,
    error_count: AtomicU32,
    disabled: AtomicBool,
}

impl ProviderExt {
    pub fn is_available(&self) -> bool {
        !self.disabled.load(Ordering::Relaxed)
    }

    /// 连续错误计数；达到阈值置 disabled，返回是否刚刚被禁用。
    fn record_error(&self, threshold: u32) -> bool {
        let n = self.error_count.fetch_add(1, Ordering::Relaxed) + 1;
        if n >= threshold && !self.disabled.swap(true, Ordering::Relaxed) {
            return true;
        }
        false
    }

    /// 成功即自愈：清零连续错误并解除自动禁用（sweep 持续轮询的场景下，
    /// 厂商恢复后扩展应自动回到可用，无需人工 reload）。
    fn record_success(&self) {
        self.error_count.store(0, Ordering::Relaxed);
        self.disabled.store(false, Ordering::Relaxed);
    }
}

/// reload 结果报告（admin reload 端点/启动日志消费）。
#[derive(Debug, Default, serde::Serialize)]
pub struct ReloadReport {
    pub loaded: usize,
    pub errors: Vec<String>,
}

/// provider 扩展注册表：key → 扩展。
pub struct ProviderExtRegistry {
    runner: StdArc<ProviderRunner>,
    builtin_keys: Vec<String>,
    entries: std::sync::RwLock<BTreeMap<String, StdArc<ProviderExt>>>,
    dir: std::sync::RwLock<Option<PathBuf>>,
    /// host 侧通用 seconds 上限（§5.4，仅约束扩展路径）。
    pub max_seconds: AtomicI64,
}

impl ProviderExtRegistry {
    /// builtin_keys：内核已占用的 provider key（Rust 原生 + OpenAI-compat
    /// 预设）——扩展与之冲突时拒绝注册（内置优先，§4.1）。
    pub fn new(builtin_keys: Vec<String>) -> Self {
        Self {
            runner: StdArc::new(ProviderRunner::new()),
            builtin_keys,
            entries: std::sync::RwLock::new(BTreeMap::new()),
            dir: std::sync::RwLock::new(None),
            max_seconds: AtomicI64::new(600),
        }
    }

    pub fn configure_dir(&self, dir: impl Into<PathBuf>) {
        *self.dir.write().expect("provider ext dir lock") = Some(dir.into());
    }

    pub fn dir(&self) -> Option<PathBuf> {
        self.dir.read().expect("provider ext dir lock").clone()
    }

    /// 执行器句柄（桥接层经此调用契约函数）。
    pub fn runner(&self) -> &ProviderRunner {
        &self.runner
    }

    pub fn get(&self, key: &str) -> Option<StdArc<ProviderExt>> {
        self.entries
            .read()
            .expect("provider ext lock")
            .get(key)
            .cloned()
    }

    pub fn list(&self) -> Vec<StdArc<ProviderExt>> {
        self.entries
            .read()
            .expect("provider ext lock")
            .values()
            .cloned()
            .collect()
    }

    pub fn max_seconds(&self) -> i64 {
        self.max_seconds.load(Ordering::Relaxed)
    }

    pub fn set_max_seconds(&self, v: i64) {
        self.max_seconds.store(v, Ordering::Relaxed);
    }

    /// 成功 → 清零连续错误；RunnerError → 累计，达阈值自动禁用（返回
    /// 是否刚被禁用，桥接层记 warn）。
    pub fn record_result(&self, key: &str, ok: bool) -> bool {
        let Some(ext) = self.get(key) else {
            return false;
        };
        if ok {
            ext.record_success();
            return false;
        }
        ext.record_error(AUTO_DISABLE_THRESHOLD)
    }

    /// 扫描目录 → fail-fast 校验 → 原子换表（reload 幂等，可反复调用）。
    pub async fn reload(&self) -> Result<ReloadReport, String> {
        let Some(dir) = self.dir() else {
            return Ok(ReloadReport::default());
        };
        let mut report = ReloadReport::default();
        let mut next: BTreeMap<String, StdArc<ProviderExt>> = BTreeMap::new();

        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(err) => {
                // 目录不存在（未部署扩展）是合法状态——静默空表。
                if dir.exists() {
                    return Err(format!("read provider ext dir {}: {err}", dir.display()));
                }
                *self.entries.write().expect("provider ext lock") = next;
                return Ok(report);
            }
        };

        for entry in entries.flatten() {
            let path = entry.path();
            let is_file = entry.file_type().is_ok_and(|ft| ft.is_file());
            if !is_file || !path.extension().is_some_and(|e| e == "js") {
                continue; // 子目录（test/）与 .test.mjs 天然跳过
            }
            let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if file_name.ends_with(".test.mjs") || file_name.ends_with(".test.js") {
                continue;
            }
            match self.load_one(&path).await {
                Ok(ext) => {
                    if self.builtin_keys.iter().any(|k| k == &ext.key) {
                        report.errors.push(format!(
                            "{}: provider key {:?} conflicts with a built-in provider (rejected)",
                            path.display(),
                            ext.key
                        ));
                        continue;
                    }
                    if next.contains_key(&ext.key) {
                        report.errors.push(format!(
                            "{}: duplicate provider key {:?} (first registration wins)",
                            path.display(),
                            ext.key
                        ));
                        continue;
                    }
                    if ext.http.is_empty() {
                        report.errors.push(format!(
                            "{}: meta.http is empty — every outbound request would be denied",
                            path.display()
                        ));
                        continue;
                    }
                    report.loaded += 1;
                    next.insert(ext.key.clone(), StdArc::new(ext));
                }
                Err(err) => report.errors.push(format!("{}: {err}", path.display())),
            }
        }

        *self.entries.write().expect("provider ext lock") = next;
        Ok(report)
    }

    /// 校验 + 构造单个扩展（meta 完整性/contract/导出存在——fail-fast）。
    async fn load_one(&self, path: &Path) -> Result<ProviderExt, String> {
        let code = std::fs::read_to_string(path).map_err(|e| format!("read: {e}"))?;
        let key_hint = path
            .file_stem()
            .and_then(|n| n.to_str())
            .unwrap_or("provider")
            .to_string();
        let err = |msg: &str| format!("{key_hint}: {msg}");

        let meta = self
            .runner
            .read_meta(&key_hint, &code, DEFAULT_TIMEOUT_MS)
            .await
            .map_err(|e| err(&runner_message(e)))?;

        let key = meta_string(&meta, "key").ok_or_else(|| err("meta.key is required"))?;
        if key.is_empty() {
            return Err(err("meta.key must not be empty"));
        }
        let contract = meta
            .get("contract")
            .and_then(Value::as_u64)
            .ok_or_else(|| err("meta.contract is required"))?;
        if contract != u64::from(SUPPORTED_CONTRACT) {
            return Err(err(&format!(
                "unsupported contract {contract} (kernel supports {SUPPORTED_CONTRACT})"
            )));
        }
        let protocols: Vec<String> = meta
            .get("protocols")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_else(|| vec!["video".to_string()]);
        if protocols.is_empty() {
            return Err(err("meta.protocols must not be empty"));
        }
        // 按声明收集必需导出函数；未知协议 → 拒绝（fail-fast）。
        let mut required: Vec<&str> = Vec::new();
        for proto in &protocols {
            let Some((_, funcs)) = PROTOCOL_FUNCS.iter().find(|(k, _)| k == proto) else {
                return Err(err(&format!("unsupported protocol {proto:?}")));
            };
            required.extend(funcs.iter().copied());
        }
        self.runner
            .validate(&key_hint, &code, &required, DEFAULT_TIMEOUT_MS)
            .await
            .map_err(|e| err(&runner_message(e)))?;

        Ok(ProviderExt {
            key,
            name: meta_string(&meta, "name").unwrap_or_else(|| key_hint.clone()),
            version: meta_string(&meta, "version").unwrap_or_default(),
            contract: contract as u32,
            protocols,
            http: meta
                .get("http")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default(),
            timeout_ms: meta
                .get("timeout_ms")
                .and_then(Value::as_u64)
                .unwrap_or(DEFAULT_TIMEOUT_MS),
            models: meta
                .get("models")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default(),
            description: meta_string(&meta, "description").unwrap_or_default(),
            file: path.to_path_buf(),
            code: StdArc::new(code),
            error_count: AtomicU32::new(0),
            disabled: AtomicBool::new(false),
        })
    }
}

fn meta_string(meta: &Value, field: &str) -> Option<String> {
    meta.get(field).and_then(Value::as_str).map(str::to_string)
}

fn runner_message(e: RunnerError) -> String {
    e.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD_JS: &str = r#"
export const meta = { key: "demo", name: "Demo", version: "0.1.0", contract: 1, protocols: ["video"], http: ["demo.example/*"], timeout_ms: 5000 };
export function buildSubmitRequest(ctx) { return { url: ctx.baseUrl + "/go", body: {} }; }
export function parseSubmitResponse(ctx, r) { return { taskId: "t" }; }
export function buildQueryRequest(ctx) { return { url: ctx.baseUrl + "/q" }; }
export function parseTaskResult(ctx, r) { return { status: "completed", url: r.body.url }; }
"#;

    fn write_file(dir: &Path, name: &str, content: impl std::fmt::Display) {
        std::fs::write(dir.join(name), content.to_string()).unwrap();
    }

    async fn scan_into(dir: &Path, builtin: &[&str]) -> StdArc<ProviderExtRegistry> {
        let reg = StdArc::new(ProviderExtRegistry::new(
            builtin.iter().map(|s| s.to_string()).collect(),
        ));
        reg.configure_dir(dir);
        reg.reload().await.unwrap();
        reg
    }

    #[tokio::test]
    async fn scan_loads_valid_extension_and_skips_tests() {
        let tmp = tempfile::tempdir().unwrap();
        write_file(tmp.path(), "demo.js", GOOD_JS);
        write_file(tmp.path(), "demo.test.mjs", "export const meta = {};"); // dev-only，跳过
        let reg = scan_into(tmp.path(), &[]).await;

        let ext = reg.get("demo").expect("demo loaded");
        assert_eq!(ext.name, "Demo");
        assert_eq!(ext.timeout_ms, 5000);
        assert!(ext.is_available());
        assert!(reg.get("demo.test").is_none());
        assert_eq!(reg.list().len(), 1);
    }

    #[tokio::test]
    async fn scan_reports_errors_without_blocking_others() {
        let tmp = tempfile::tempdir().unwrap();
        write_file(tmp.path(), "good.js", GOOD_JS);
        write_file(tmp.path(), "nometa.js", "export function f(){}");
        write_file(
            tmp.path(),
            "contract.js",
            GOOD_JS.replace("contract: 1", "contract: 2"),
        );
        let reg = scan_into(tmp.path(), &[]).await;

        assert!(reg.get("demo").is_some(), "good one loads");
        assert!(reg.get("nometa").is_none());
        assert!(reg.get("contract").is_none());
    }

    #[tokio::test]
    async fn builtin_key_conflict_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        write_file(tmp.path(), "kling.js", GOOD_JS.replace("demo", "kling"));
        let reg = scan_into(tmp.path(), &["kling"]).await;
        assert!(reg.get("kling").is_none(), "builtin wins");
    }

    #[tokio::test]
    async fn duplicate_key_first_wins() {
        let tmp = tempfile::tempdir().unwrap();
        write_file(tmp.path(), "a.js", GOOD_JS.replace("Demo", "A"));
        write_file(tmp.path(), "b.js", GOOD_JS.replace("Demo", "B"));
        let reg = scan_into(tmp.path(), &[]).await;
        let ext = reg.get("demo").unwrap();
        assert_eq!(ext.name, "A", "first file (sorted) wins");
    }

    #[tokio::test]
    async fn empty_http_allowlist_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        write_file(
            tmp.path(),
            "nohttp.js",
            GOOD_JS.replace(r#"http: ["demo.example/*"], "#, ""),
        );
        let reg = scan_into(tmp.path(), &[]).await;
        assert!(reg.get("demo").is_none());
    }

    #[tokio::test]
    async fn reload_picks_up_new_files_and_removals() {
        let tmp = tempfile::tempdir().unwrap();
        let reg = scan_into(tmp.path(), &[]).await;
        assert!(reg.get("demo").is_none());

        write_file(tmp.path(), "demo.js", GOOD_JS);
        reg.reload().await.unwrap();
        assert!(reg.get("demo").is_some());

        std::fs::remove_file(tmp.path().join("demo.js")).unwrap();
        reg.reload().await.unwrap();
        assert!(reg.get("demo").is_none(), "removal clears entry");
    }

    #[tokio::test]
    async fn consecutive_errors_auto_disable_then_success_recovers() {
        let tmp = tempfile::tempdir().unwrap();
        write_file(tmp.path(), "demo.js", GOOD_JS);
        let reg = scan_into(tmp.path(), &[]).await;

        assert!(reg.get("demo").unwrap().is_available());
        assert!(!reg.record_result("demo", false));
        assert!(!reg.record_result("demo", false));
        assert!(reg.record_result("demo", false), "3rd error disables");
        assert!(!reg.get("demo").unwrap().is_available());

        assert!(!reg.record_result("demo", true), "success resets");
        assert!(reg.get("demo").unwrap().is_available());
    }
}
