use serde::Serialize;
#[cfg(feature = "export-types")]
use ts_rs::TS;
use utoipa::ToSchema;

use crate::models::media::Media;
use crate::utils::tz::Timestamp;

/// Media file API response
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, ToSchema)]
pub struct MediaResponse {
    pub id: String,
    pub filename: String,
    pub url: String,
    pub mimetype: String,
    pub size: i64,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub title: Option<String>,
    pub alt_text: Option<String>,
    pub caption: Option<String>,
    pub description: Option<String>,
    #[schema(value_type = String)]
    pub created_at: Timestamp,
    #[schema(value_type = String)]
    pub updated_at: Timestamp,
}

/// Convert Media database model to API response
#[must_use]
pub fn media_to_response(media: &Media, base_url: &str) -> MediaResponse {
    MediaResponse {
        id: media.id.to_string(),
        filename: media.filename.clone(),
        url: format!("{}/uploads/{}", base_url, media.filepath),
        mimetype: media.mimetype.clone(),
        size: media.size,
        width: media.width,
        height: media.height,
        title: media.title.clone(),
        alt_text: media.alt_text.clone(),
        caption: media.caption.clone(),
        description: media.description.clone(),
        created_at: media.created_at,
        updated_at: media.updated_at,
    }
}

/// Convert Media model to API response (using URL provided by Storage layer)
#[must_use]
pub fn media_to_response_with_url(media: &Media, url: &str) -> MediaResponse {
    MediaResponse {
        id: media.id.to_string(),
        filename: media.filename.clone(),
        url: url.to_string(),
        mimetype: media.mimetype.clone(),
        size: media.size,
        width: media.width,
        height: media.height,
        title: media.title.clone(),
        alt_text: media.alt_text.clone(),
        caption: media.caption.clone(),
        description: media.description.clone(),
        created_at: media.created_at,
        updated_at: media.updated_at,
    }
}

/// Storage statistics API response
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, ToSchema)]
pub struct MediaStatsResponse {
    pub total_files: i64,
    pub total_size: i64,
    pub by_type: Vec<MediaTypeInfoResponse>,
}

/// Statistics grouped by MIME type
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, ToSchema)]
pub struct MediaTypeInfoResponse {
    pub mimetype: String,
    pub count: i64,
    pub total_size: i64,
}

/// Convert MediaStats model to API response
#[must_use]
pub fn stats_to_response(stats: &crate::models::media::MediaStats) -> MediaStatsResponse {
    MediaStatsResponse {
        total_files: stats.total_files,
        total_size: stats.total_size,
        by_type: stats
            .by_type
            .iter()
            .map(|t| MediaTypeInfoResponse {
                mimetype: t.mimetype.clone(),
                count: t.count,
                total_size: t.total_size,
            })
            .collect(),
    }
}
