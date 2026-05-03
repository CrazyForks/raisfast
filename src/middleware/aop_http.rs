//! HTTP AOP 中间件
//!
//! 在请求处理前后调用 AspectEngine 的 HTTP Layer dispatch，
//! 允许 aspect 拦截/修改请求和响应。

use std::collections::HashMap;

use axum::body::Body;
use axum::extract::State;
use axum::http::{Request, Response};
use axum::middleware::Next;
use axum::response::IntoResponse;

use crate::aspects::{BaseContext, HttpAfterContext, HttpBeforeContext};
use crate::AppState;

pub async fn aop_http_layer(
    State(state): State<AppState>,
    req: Request<Body>,
    next: Next,
) -> impl IntoResponse {
    let method = req.method().to_string();
    let path = req.uri().path().to_string();
    let headers: HashMap<String, String> = req
        .headers()
        .iter()
        .filter_map(|(k, v)| v.to_str().ok().map(|s| (k.to_string(), s.to_string())))
        .collect();

    let mut ctx = HttpBeforeContext {
        base: BaseContext::new(None, "default".to_string(), chrono::Utc::now().to_rfc3339()),
        method,
        path: path.clone(),
        headers,
    };

    match state
        .aspect_engine
        .dispatch_http_before(&path, &mut ctx)
        .await
    {
        Ok(Some(val)) => {
            let body = serde_json::to_string(&val).unwrap_or_else(|_| "{}".to_string());
            return Response::builder()
                .status(200)
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap();
        }
        Ok(None) => {}
        Err(e) => {
            tracing::warn!("aop http before dispatch error: {e}");
        }
    }

    let response = next.run(req).await;
    let status_code = response.status().as_u16();

    let mut after_ctx = HttpAfterContext {
        base: ctx.base.clone(),
        status_code,
        response_body: None,
    };

    if let Err(e) = state
        .aspect_engine
        .dispatch_http_after(&path, &mut after_ctx)
        .await
    {
        tracing::warn!("aop http after dispatch error: {e}");
    }

    response
}
