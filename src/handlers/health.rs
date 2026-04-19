//! 健康检查处理器
//!
//! 提供两个端点：
//!
//! - `GET /healthz` — 存活探针（liveness），Kubernetes 判断进程是否需要重启
//! - `GET /readyz`  — 就绪探针（readiness），Kubernetes 判断是否可以接收流量
//!
//! `/health` 保留为兼容旧版，行为与 `/readyz` 相同。

use axum::Json;
use axum::extract::State;
use serde_json::{Value, json};

use crate::errors::response::ApiResponse;

/// 存活探针
///
/// 仅检查进程存活，不检查外部依赖。
/// 若此端点失败，Kubernetes 会重启 Pod。
#[utoipa::path(get, path = "/healthz", tag = "health",
    responses((status = 200, description = "进程存活"))
)]
pub async fn liveness() -> Json<ApiResponse<Value>> {
    Json(ApiResponse::success(json!({"status": "alive"})))
}

/// 就绪探针
///
/// 检查数据库连通性，确认服务已准备好接收流量。
/// 若此端点失败，Kubernetes 会将 Pod 从 Service Endpoints 中移除（但不重启）。
#[utoipa::path(get, path = "/readyz", tag = "health",
    responses((status = 200, description = "服务就绪"))
)]
pub async fn readiness(
    State(state): State<crate::AppState>,
) -> Result<Json<ApiResponse<Value>>, (axum::http::StatusCode, Json<Value>)> {
    let db_ok = sqlx::query("SELECT 1").execute(&state.pool).await.is_ok();

    if db_ok {
        Ok(Json(ApiResponse::success(json!({
            "status": "ready",
            "db": "ok"
        }))))
    } else {
        tracing::error!("readiness check failed: database unreachable");
        Err((
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({
                "code": 50300,
                "message": "database unavailable",
                "data": null
            })),
        ))
    }
}

/// 兼容旧版健康检查（行为等同于 readiness）
#[utoipa::path(get, path = "/health", tag = "health",
    responses((status = 200, description = "健康检查通过"))
)]
pub async fn health(
    State(state): State<crate::AppState>,
) -> Result<Json<ApiResponse<Value>>, (axum::http::StatusCode, Json<Value>)> {
    let db_ok = sqlx::query("SELECT 1").execute(&state.pool).await.is_ok();

    if db_ok {
        Ok(Json(ApiResponse::success(json!({
            "status": "ok",
            "db": "ok"
        }))))
    } else {
        tracing::error!("health check failed: database unreachable");
        Err((
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({
                "code": 50300,
                "message": "database unavailable",
                "data": null
            })),
        ))
    }
}
