//! HTTP AOP middleware
//!
//! Calls AspectEngine's HTTP Layer dispatch before and after request processing,
//! allowing aspects to intercept/modify requests and responses.

use std::collections::HashMap;

use axum::body::Body;
use axum::extract::State;
use axum::http::{Request, Response};
use axum::middleware::Next;
use axum::response::IntoResponse;

use crate::AppState;
use crate::aspects::{BaseContext, HttpAfterContext, HttpBeforeContext};

pub async fn aop_http_layer(
    State(state): State<AppState>,
    req: Request<Body>,
    next: Next,
) -> impl IntoResponse {
    let path = req.uri().path();

    let skip = matches!(
        path,
        "/health" | "/healthz" | "/readyz" | "/metrics" | "/feed.xml"
    );
    if skip {
        return next.run(req).await;
    }

    let method = req.method().to_string();
    let path_owned = path.to_string();
    let headers: HashMap<String, String> = req
        .headers()
        .iter()
        .filter_map(|(k, v)| v.to_str().ok().map(|s| (k.to_string(), s.to_string())))
        .collect();

    let mut ctx = HttpBeforeContext {
        base: BaseContext::new(None, "default".to_string(), chrono::Utc::now().to_rfc3339()),
        method,
        path: path_owned.clone(),
        headers,
    };

    match state
        .aspect_engine
        .dispatch_http_before(&path_owned, &mut ctx)
        .await
    {
        Ok(Some(val)) => {
            let body = serde_json::to_string(&val).unwrap_or_else(|_| "{}".to_string());
            return Response::builder()
                .status(200)
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap_or_else(|_| Response::new(Body::from(r#"{"code":0,"data":null}"#)));
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
        .dispatch_http_after(&path_owned, &mut after_ctx)
        .await
    {
        tracing::warn!("aop http after dispatch error: {e}");
    }

    response
}
