//! 健康检查处理器
//!
//! 提供服务健康状态检查接口，用于负载均衡和监控。
//! 会同时检查 HTTP 服务和数据库连接。

use axum::Json;
use axum::extract::State;
use serde_json::{Value, json};

use crate::errors::response::ApiResponse;

/// 健康检查
///
/// - **方法/路径：** `GET /api/v1/health`
/// - **认证：** 无需认证
/// - **说明：** 返回服务运行状态，包括数据库连通性检测。
///   若数据库不可达，返回 503。
/// - **返回：** `ApiResponse<Value>`（包含 `{"status": "ok", "db": "ok"}`）
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
