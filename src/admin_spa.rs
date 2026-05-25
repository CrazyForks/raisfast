use axum::{
    http::{StatusCode, Uri, header},
    response::{IntoResponse, Response},
};
use rust_embed::Embed;

#[derive(Embed)]
#[folder = "adminui/"]
#[prefix = ""]
#[exclude = ".git"]
struct AdminAssets;

pub async fn serve_admin(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');

    if path.is_empty() || path == "/" {
        return match AdminAssets::get("index.html") {
            Some(content) => {
                let body = content.data;
                let mime = "text/html; charset=utf-8";
                (StatusCode::OK, [(header::CONTENT_TYPE, mime)], body).into_response()
            }
            None => StatusCode::NOT_FOUND.into_response(),
        };
    }

    if let Some(content) = AdminAssets::get(path) {
        let mime = content.metadata.mimetype();
        return (StatusCode::OK, [(header::CONTENT_TYPE, mime)], content.data).into_response();
    }

    match AdminAssets::get("index.html") {
        Some(content) => {
            let body = content.data;
            let mime = "text/html; charset=utf-8";
            (StatusCode::OK, [(header::CONTENT_TYPE, mime)], body).into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

pub async fn serve_admin_asset(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');

    let asset_path = if path.starts_with("assets/") {
        path.to_string()
    } else {
        format!("assets/{path}")
    };

    match AdminAssets::get(&asset_path) {
        Some(content) => {
            let mime = content.metadata.mimetype();
            (StatusCode::OK, [(header::CONTENT_TYPE, mime)], content.data).into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}
