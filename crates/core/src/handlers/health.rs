//! Health check handlers with Actuator-style component checks.
//!
//! Provides three endpoints:
//!
//! - `GET /healthz` — liveness probe (process only, no external deps)
//! - `GET /readyz`  — readiness probe (all component checks)
//! - `GET /health`  — full health report with component details

use std::collections::HashMap;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde_json::{Value, json};

use crate::dto::{ComponentHealth, HealthResponse, HealthStatus};
use crate::errors::response::ApiResponse;

// ── HealthIndicator trait ───────────────────────────────────

/// A component that can report its health status.
pub trait HealthIndicator: Send + Sync {
    fn name(&self) -> &str;
    fn check(
        &self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ComponentHealth> + Send + '_>>;
}

// ── Built-in indicators ────────────────────────────────────

struct DatabaseIndicator {
    pool: crate::db::Pool,
}

impl HealthIndicator for DatabaseIndicator {
    fn name(&self) -> &str {
        "database"
    }
    fn check(
        &self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ComponentHealth> + Send + '_>> {
        Box::pin(async {
            match sqlx::query("SELECT 1").execute(&self.pool).await {
                Ok(_r) => {
                    let _: crate::db::pool::DbQueryResult = _r;
                    ComponentHealth {
                        status: HealthStatus::Up,
                        details: None,
                    }
                }
                Err(e) => ComponentHealth {
                    status: HealthStatus::Down,
                    details: Some({
                        let mut m = serde_json::Map::new();
                        m.insert("error".into(), serde_json::Value::String(e.to_string()));
                        serde_json::Value::Object(m)
                    }),
                },
            }
        })
    }
}

struct StorageIndicator {
    storage_root: String,
}

impl HealthIndicator for StorageIndicator {
    fn name(&self) -> &str {
        "storage"
    }
    fn check(
        &self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ComponentHealth> + Send + '_>> {
        let root = self.storage_root.clone();
        Box::pin(async move {
            let path = std::path::Path::new(&root);
            let writable = std::fs::create_dir_all(path).is_ok() && {
                let test_file = path.join(".health_check");
                let can_write = std::fs::write(&test_file, b"ok").is_ok();
                let _ = std::fs::remove_file(&test_file);
                can_write
            };

            if writable {
                ComponentHealth {
                    status: HealthStatus::Up,
                    details: Some(json!({ "path": root })),
                }
            } else {
                ComponentHealth {
                    status: HealthStatus::Down,
                    details: Some(json!({ "path": root, "error": "not writable" })),
                }
            }
        })
    }
}

struct SearchIndicator {
    search: std::sync::Arc<dyn crate::search::SearchEngine>,
}

impl HealthIndicator for SearchIndicator {
    fn name(&self) -> &str {
        "search"
    }
    fn check(
        &self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ComponentHealth> + Send + '_>> {
        Box::pin(async {
            ComponentHealth {
                status: HealthStatus::Up,
                details: Some(json!({ "engine": self.search.engine_name() })),
            }
        })
    }
}

struct CacheIndicator {
    cache: std::sync::Arc<dyn crate::cache::CacheStore>,
}

impl HealthIndicator for CacheIndicator {
    fn name(&self) -> &str {
        "cache"
    }
    fn check(
        &self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ComponentHealth> + Send + '_>> {
        Box::pin(async {
            match self
                .cache
                .set("__health__", "ok", Some(std::time::Duration::from_secs(1)))
                .await
            {
                Ok(_) => {
                    let _ = self.cache.delete("__health__").await;
                    ComponentHealth {
                        status: HealthStatus::Up,
                        details: None,
                    }
                }
                Err(e) => ComponentHealth {
                    status: HealthStatus::Degraded,
                    details: Some(json!({ "error": e.to_string() }) as Value),
                },
            }
        })
    }
}

// ── Build indicators ────────────────────────────────────────

fn build_indicators(state: &crate::AppState) -> Vec<Box<dyn HealthIndicator>> {
    vec![
        Box::new(DatabaseIndicator {
            pool: state.pool.clone(),
        }),
        Box::new(StorageIndicator {
            storage_root: state.config.storage_root_dir.clone(),
        }),
        Box::new(SearchIndicator {
            search: state.search.clone(),
        }),
        Box::new(CacheIndicator {
            cache: state.cache.clone(),
        }),
    ]
}

async fn run_checks(state: &crate::AppState) -> HealthResponse {
    let indicators = build_indicators(state);
    let mut components = HashMap::new();
    let mut overall = HealthStatus::Up;

    for indicator in &indicators {
        let health = indicator.check().await;
        if health.status == HealthStatus::Down {
            overall = HealthStatus::Down;
        } else if health.status == HealthStatus::Degraded && overall != HealthStatus::Down {
            overall = HealthStatus::Degraded;
        }
        components.insert(indicator.name().to_string(), health);
    }

    let uptime = state
        .config
        .started_at
        .map(|t| t.elapsed().as_secs())
        .unwrap_or(0);

    HealthResponse {
        status: overall,
        components,
        uptime_seconds: uptime,
        version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

// ── Handlers ────────────────────────────────────────────────

/// Liveness probe — process only, no external deps.
#[utoipa::path(get, path = "/healthz", tag = "health",
    responses((status = 200, description = "process alive"))
)]
pub async fn liveness() -> Json<ApiResponse<Value>> {
    Json(ApiResponse::success(json!({"status": "alive"})))
}

/// Readiness probe — all component checks.
#[utoipa::path(get, path = "/readyz", tag = "health",
    responses((status = 200, description = "service ready"))
)]
pub async fn readiness(
    State(state): State<crate::AppState>,
) -> Result<Json<ApiResponse<Value>>, (StatusCode, Json<Value>)> {
    let report = run_checks(&state).await;
    if report.status == HealthStatus::Down {
        let details = serde_json::to_value(&report).unwrap_or_default();
        Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({
                "code": 50300,
                "message": "service unavailable",
                "data": details
            })),
        ))
    } else {
        Ok(Json(ApiResponse::success(
            serde_json::to_value(&report).unwrap_or_default(),
        )))
    }
}

/// Full health report with component details.
#[utoipa::path(get, path = "/health", tag = "health",
    responses((status = 200, description = "full health report"))
)]
pub async fn health(
    State(state): State<crate::AppState>,
) -> Result<Json<ApiResponse<Value>>, (StatusCode, Json<Value>)> {
    readiness(State(state)).await
}

// ── Tests ───────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_status_serializes_uppercase() {
        let s = serde_json::to_string(&HealthStatus::Up).unwrap();
        assert_eq!(s, "\"UP\"");
        let s = serde_json::to_string(&HealthStatus::Down).unwrap();
        assert_eq!(s, "\"DOWN\"");
        let s = serde_json::to_string(&HealthStatus::Degraded).unwrap();
        assert_eq!(s, "\"DEGRADED\"");
    }

    #[test]
    fn health_response_structure() {
        let mut components = HashMap::new();
        components.insert(
            "database".to_string(),
            ComponentHealth {
                status: HealthStatus::Up,
                details: None,
            },
        );
        let resp = HealthResponse {
            status: HealthStatus::Up,
            components,
            uptime_seconds: 42,
            version: "0.1.0".to_string(),
        };
        let val = serde_json::to_value(&resp).unwrap();
        assert_eq!(val["status"], "UP");
        assert_eq!(val["uptime_seconds"], 42);
        assert_eq!(val["version"], "0.1.0");
        assert_eq!(val["components"]["database"]["status"], "UP");
    }
}
