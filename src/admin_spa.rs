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
