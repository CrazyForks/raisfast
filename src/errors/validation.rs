//! 输入校验与 i18n 翻译桥接
//!
//! 本模块封装了 `validator` crate 的校验逻辑，将校验错误自动翻译为
//! 当前 locale 对应的自然语言消息，替代了手动调用 `req.validate().map_err(...)`
//! 的繁琐模式。
//!
//! # 核心函数
//!
//! - [`validate`]：对任意实现了 `validator::Validate` 的结构体执行校验并翻译错误
//! - [`translate_errors`]：将 `ValidationErrors` 翻译为本地化的错误消息列表
//! - [`translate_field`]：将字段名翻译为当前 locale 的显示名称

use crate::errors::app_error::{AppError, AppResult};
use validator::ValidationErrors;

/// 执行输入校验并自动翻译错误
///
/// 对传入的请求结构体执行 `validator::Validate::validate()`，若校验通过返回 `Ok(())`，
/// 若失败则通过 [`translate_errors`] 将所有校验错误翻译为本地化消息，
/// 并合并为一个 [`AppError::BadRequest`] 返回。
///
/// # 参数
///
/// - `req` — 实现了 `validator::Validate` trait 的请求结构体引用
///
/// # 返回值
///
/// - `Ok(())` — 校验通过
/// - `Err(AppError::BadRequest)` — 校验失败，消息为所有错误的分号连接字符串
///
/// # 替代模式
///
/// 替换了旧的手动模式：
///
/// ```ignore
/// // 旧写法（繁琐且不统一）
/// req.validate().map_err(|e| AppError::BadRequest(e.to_string()))?;
///
/// // 新写法（简洁且支持 i18n）
/// validate(&req)?;
/// ```
pub fn validate(req: &dyn validator::Validate) -> AppResult<()> {
    match req.validate() {
        Ok(()) => Ok(()),
        Err(errors) => translate_errors(&errors),
    }
}

/// 翻译校验错误为本地化消息列表
///
/// 遍历 `ValidationErrors` 中的每个字段及其错误列表，根据错误类型
/// 选择对应的 i18n 翻译键，并将字段名通过 [`translate_field`] 翻译为
/// 当前 locale 的显示名称。
///
/// # i18n 键映射
///
/// | 校验规则        | i18n 键                    | 参数                        |
/// |----------------|---------------------------|-----------------------------|
/// | `required`     | `validation.required`     | `field`                     |
/// | `length` (范围) | `validation.length_range` | `field`, `min`, `max`       |
/// | `length` (最小) | `validation.min_length`   | `field`, `min`              |
/// | `length` (最大) | `validation.max_length`   | `field`, `max`              |
/// | `email`        | `validation.email_invalid`| —                           |
/// | 其他            | `validation.required`     | `field`（兜底）              |
///
/// # 多错误处理
///
/// 当同一字段存在多个校验错误时，所有翻译后的消息以分号 `;` 连接，
/// 作为 `BadRequest` 的描述信息一次性返回给客户端。
fn translate_errors(errors: &ValidationErrors) -> AppResult<()> {
    let locale = crate::middleware::locale::current_locale();
    rust_i18n::set_locale(&locale);
    let mut messages: Vec<String> = Vec::new();

    for (field, field_errors) in errors.field_errors() {
        let field_name = translate_field(field);
        for error in field_errors {
            let msg = match error.code.as_ref() {
                "required" | "length" => {
                    let min = error
                        .params
                        .get("min")
                        .and_then(serde_json::Value::as_u64)
                        .map(|v| v.to_string());
                    let max = error
                        .params
                        .get("max")
                        .and_then(serde_json::Value::as_u64)
                        .map(|v| v.to_string());
                    let exact = error
                        .params
                        .get("value")
                        .and_then(serde_json::Value::as_u64)
                        .map(|v| v.to_string());

                    match (min.as_deref(), max.as_deref()) {
                        (Some(min), Some(max)) if min != max => rust_i18n::t!(
                            "validation.length_range",
                            field = field_name,
                            min = min,
                            max = max
                        )
                        .to_string(),
                        (Some(min), Some(_)) => {
                            rust_i18n::t!("validation.min_length", field = field_name, min = min)
                                .to_string()
                        }
                        (Some(min), None) => {
                            rust_i18n::t!("validation.min_length", field = field_name, min = min)
                                .to_string()
                        }
                        (None, Some(max)) => {
                            rust_i18n::t!("validation.max_length", field = field_name, max = max)
                                .to_string()
                        }
                        _ => {
                            if let Some(v) = exact {
                                rust_i18n::t!("validation.min_length", field = field_name, min = v)
                                    .to_string()
                            } else {
                                rust_i18n::t!("validation.required", field = field_name).to_string()
                            }
                        }
                    }
                }
                "email" => rust_i18n::t!("validation.email_invalid").to_string(),
                _ => rust_i18n::t!("validation.required", field = field_name).to_string(),
            };
            messages.push(msg);
        }
    }

    Err(AppError::BadRequest(messages.join("; ")))
}

/// 翻译字段名为当前 locale 的显示名称
///
/// 通过 i18n 键 `fields.{field_name}` 查找字段名的本地化翻译。
/// 例如，字段 `email` 在中文环境下查找 `fields.email` 键得到 `"邮箱"`。
///
/// # 参数
///
/// - `field` — 结构体中的原始字段名（如 `"email"`、`"password"`）
/// - `locale` — 目标语言标识
///
/// # 返回值
///
/// - 若 `fields.{field}` 键存在，返回翻译后的字段名
/// - 若键不存在（`rust_i18n::t!` 回退为键名本身），则返回原始字段名作为兜底
fn translate_field(field: &str) -> String {
    let key = format!("fields.{field}");
    let translated = rust_i18n::t!(&key);
    if translated == key {
        field.to_string()
    } else {
        translated.to_string()
    }
}
