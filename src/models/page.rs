//! 页面与块模型及数据库查询

use chrono::Utc;
use serde::{Deserialize, Serialize};
#[cfg(feature = "export-types")]
use ts_rs::TS;

use crate::db::tenant::tenant_filter;
use crate::errors::app_error::{AppError, AppResult};

// ── 数据库行模型 ──

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Page {
    pub id: String,
    pub tenant_id: Option<String>,
    pub title: String,
    pub slug: String,
    pub content: Option<String>,
    pub blocks: Option<String>,
    pub meta_title: Option<String>,
    pub meta_description: Option<String>,
    pub og_image: Option<String>,
    pub template: String,
    pub parent_id: Option<String>,
    pub sort_order: i64,
    pub status: String,
    pub created_by: String,
    pub updated_by: Option<String>,
    pub cover_image: Option<String>,
    pub published_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

crate::impl_from_row_opt_tenant!(Page {
    required { id, title, slug, template, sort_order, status, created_by, created_at, updated_at }
    optional { content, blocks, meta_title, meta_description, og_image, parent_id, updated_by, cover_image, published_at }
});

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ReusableBlock {
    pub id: String,
    pub tenant_id: Option<String>,
    pub name: String,
    pub block_type: String,
    pub content: String,
    pub description: Option<String>,
    pub created_by: Option<String>,
    pub updated_by: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

crate::impl_from_row_opt_tenant!(ReusableBlock {
    required { id, name, block_type, content, created_at, updated_at }
    optional { description, created_by, updated_by }
});

// ── Block 类型系统 ──

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[cfg_attr(feature = "export-types", ts(rename_all = "snake_case"))]
pub enum PageBlock {
    Hero {
        title: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        subtitle: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        background_image: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        cta_text: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        cta_url: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        alignment: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        overlay: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        height: Option<String>,
    },
    Richtext {
        content: String,
    },
    Image {
        url: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        alt: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        caption: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        link: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        width: Option<String>,
    },
    Gallery {
        images: Vec<GalleryImage>,
        #[serde(skip_serializing_if = "Option::is_none")]
        columns: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        gap: Option<String>,
    },
    Video {
        url: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        provider: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        autoplay: Option<bool>,
    },
    Cta {
        title: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        button_text: String,
        button_url: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        style: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        background_image: Option<String>,
    },
    Testimonial {
        items: Vec<TestimonialItem>,
        #[serde(skip_serializing_if = "Option::is_none")]
        layout: Option<String>,
    },
    Faq {
        items: Vec<FaqItem>,
    },
    Stats {
        items: Vec<StatItem>,
        #[serde(skip_serializing_if = "Option::is_none")]
        background: Option<String>,
    },
    Timeline {
        items: Vec<TimelineItem>,
    },
    Team {
        members: Vec<TeamMember>,
        #[serde(skip_serializing_if = "Option::is_none")]
        columns: Option<u32>,
    },
    Pricing {
        plans: Vec<PricingPlan>,
        #[serde(skip_serializing_if = "Option::is_none")]
        highlight_index: Option<usize>,
    },
    ContactForm {
        #[serde(skip_serializing_if = "Option::is_none")]
        email_to: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        fields: Option<Vec<FormFieldDef>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        submit_text: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        success_message: Option<String>,
    },
    Map {
        #[serde(skip_serializing_if = "Option::is_none")]
        address: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        lat: Option<f64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        lng: Option<f64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        zoom: Option<u32>,
    },
    Code {
        code: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        language: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        show_line_numbers: Option<bool>,
    },
    Quote {
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        author: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        source: Option<String>,
    },
    Divider {
        #[serde(skip_serializing_if = "Option::is_none")]
        style: Option<String>,
    },
    Spacer {
        #[serde(skip_serializing_if = "Option::is_none")]
        height: Option<String>,
    },
    Columns {
        columns: Vec<ColumnDef>,
        #[serde(skip_serializing_if = "Option::is_none")]
        gap: Option<String>,
    },
    Html {
        content: String,
    },
    Reusable {
        ref_id: String,
    },
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GalleryImage {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caption: Option<String>,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestimonialItem {
    pub quote: String,
    pub author: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub company: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rating: Option<u32>,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FaqItem {
    pub question: String,
    pub answer: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_open: Option<bool>,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatItem {
    pub label: String,
    pub value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suffix: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineItem {
    pub date: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamMember {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bio: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub social_links: Option<Vec<SocialLink>>,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SocialLink {
    pub platform: String,
    pub url: String,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PricingPlan {
    pub name: String,
    pub price: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub features: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub button_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub button_url: Option<String>,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FormFieldDef {
    pub name: String,
    pub label: String,
    pub field_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnDef {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<String>,
    pub blocks: Vec<PageBlock>,
}

// ── 查询函数 ──

pub async fn find_by_slug(
    pool: &crate::db::Pool,
    slug: &str,
    tenant_id: Option<&str>,
) -> AppResult<Option<Page>> {
    let sql = format!(
        "SELECT * FROM pages WHERE slug = ?{}",
        tenant_filter(tenant_id)
    );
    let sql = crate::db::dialect::translate(&sql);
    let mut q = sqlx::query_as::<_, Page>(&sql).bind(slug);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    Ok(q.fetch_optional(pool).await?)
}

pub async fn find_by_id(
    pool: &crate::db::Pool,
    id: &str,
    tenant_id: Option<&str>,
) -> AppResult<Option<Page>> {
    let sql = format!(
        "SELECT * FROM pages WHERE id = ?{}",
        tenant_filter(tenant_id)
    );
    let sql = crate::db::dialect::translate(&sql);
    let mut q = sqlx::query_as::<_, Page>(&sql).bind(id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    Ok(q.fetch_optional(pool).await?)
}

pub async fn list_published(
    pool: &crate::db::Pool,
    page: i64,
    page_size: i64,
    tenant_id: Option<&str>,
) -> AppResult<(Vec<Page>, i64)> {
    let offset = (page - 1) * page_size;
    let count_sql = format!(
        "SELECT COUNT(*) FROM pages WHERE status = 'published'{}",
        tenant_filter(tenant_id)
    );
    let count_sql = crate::db::dialect::translate(&count_sql);
    let mut cq = sqlx::query_scalar::<_, i64>(&count_sql);
    if let Some(tid) = tenant_id {
        cq = cq.bind(tid);
    }
    let total = cq.fetch_one(pool).await?;

    let data_sql = format!(
        "SELECT * FROM pages WHERE status = 'published'{} ORDER BY sort_order ASC, created_at DESC LIMIT ? OFFSET ?",
        tenant_filter(tenant_id)
    );
    let data_sql = crate::db::dialect::translate(&data_sql);
    let mut dq = sqlx::query_as::<_, Page>(&data_sql);
    if let Some(tid) = tenant_id {
        dq = dq.bind(tid);
    }
    dq = dq.bind(page_size).bind(offset);
    let items = dq.fetch_all(pool).await?;
    Ok((items, total))
}

pub async fn list_all(
    pool: &crate::db::Pool,
    page: i64,
    page_size: i64,
    status: Option<&str>,
    tenant_id: Option<&str>,
) -> AppResult<(Vec<Page>, i64)> {
    let offset = (page - 1) * page_size;
    let status_clause = status.map(|_| " AND status = ?").unwrap_or("");

    let tf = tenant_filter(tenant_id);
    let count_sql = format!("SELECT COUNT(*) FROM pages WHERE 1=1{status_clause}{tf}");
    let count_sql = crate::db::dialect::translate(&count_sql);
    let mut cq = sqlx::query_scalar::<_, i64>(&count_sql);
    if let Some(s) = status {
        cq = cq.bind(s);
    }
    if let Some(tid) = tenant_id {
        cq = cq.bind(tid);
    }
    let total = cq.fetch_one(pool).await?;

    let data_sql = format!(
        "SELECT * FROM pages WHERE 1=1{status_clause}{tf} ORDER BY sort_order ASC, created_at DESC LIMIT ? OFFSET ?"
    );
    let data_sql = crate::db::dialect::translate(&data_sql);
    let mut dq = sqlx::query_as::<_, Page>(&data_sql);
    if let Some(s) = status {
        dq = dq.bind(s);
    }
    if let Some(tid) = tenant_id {
        dq = dq.bind(tid);
    }
    dq = dq.bind(page_size).bind(offset);
    let items = dq.fetch_all(pool).await?;
    Ok((items, total))
}

#[allow(clippy::too_many_arguments)]
pub async fn create(
    pool: &crate::db::Pool,
    id: &str,
    title: &str,
    slug: &str,
    content: Option<&str>,
    blocks: Option<&str>,
    meta_title: Option<&str>,
    meta_description: Option<&str>,
    og_image: Option<&str>,
    template: &str,
    parent_id: Option<&str>,
    sort_order: i64,
    status: &str,
    created_by: &str,
    cover_image: Option<&str>,
    tenant_id: Option<&str>,
) -> AppResult<Page> {
    let now = Utc::now().to_rfc3339();
    let published_at = if status == "published" {
        Some(now.clone())
    } else {
        None
    };

    match tenant_id {
        Some(tid) => {
            let sql = crate::db::dialect::translate(
                "INSERT INTO pages (id, tenant_id, title, slug, content, blocks, meta_title, meta_description, og_image, template, parent_id, sort_order, status, created_by, updated_by, cover_image, published_at, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            );
            sqlx::query(&sql)
                .bind(id)
                .bind(tid)
                .bind(title)
                .bind(slug)
                .bind(content)
                .bind(blocks)
                .bind(meta_title)
                .bind(meta_description)
                .bind(og_image)
                .bind(template)
                .bind(parent_id)
                .bind(sort_order)
                .bind(status)
                .bind(created_by)
                .bind(created_by)
                .bind(cover_image)
                .bind(&published_at)
                .bind(&now)
                .bind(&now)
                .execute(pool)
                .await?;
        }
        None => {
            let sql = crate::db::dialect::translate(
                "INSERT INTO pages (id, title, slug, content, blocks, meta_title, meta_description, og_image, template, parent_id, sort_order, status, created_by, updated_by, cover_image, published_at, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            );
            sqlx::query(&sql)
                .bind(id)
                .bind(title)
                .bind(slug)
                .bind(content)
                .bind(blocks)
                .bind(meta_title)
                .bind(meta_description)
                .bind(og_image)
                .bind(template)
                .bind(parent_id)
                .bind(sort_order)
                .bind(status)
                .bind(created_by)
                .bind(created_by)
                .bind(cover_image)
                .bind(&published_at)
                .bind(&now)
                .bind(&now)
                .execute(pool)
                .await?;
        }
    }

    find_by_id(pool, id, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found("page"))
}

#[allow(clippy::too_many_arguments)]
pub async fn update(
    pool: &crate::db::Pool,
    id: &str,
    title: Option<&str>,
    slug: Option<&str>,
    content: Option<&str>,
    blocks: Option<&str>,
    meta_title: Option<&str>,
    meta_description: Option<&str>,
    og_image: Option<&str>,
    template: Option<&str>,
    parent_id: Option<Option<&str>>,
    sort_order: Option<i64>,
    status: Option<&str>,
    cover_image: Option<&str>,
    updated_by: Option<&str>,
    tenant_id: Option<&str>,
) -> AppResult<Page> {
    let now = Utc::now().to_rfc3339();
    let mut sets = vec!["updated_at = ?".to_string()];

    if updated_by.is_some() {
        sets.push("updated_by = ?".to_string());
    }
    if title.is_some() {
        sets.push("title = ?".to_string());
    }
    if slug.is_some() {
        sets.push("slug = ?".to_string());
    }
    if content.is_some() {
        sets.push("content = ?".to_string());
    }
    if blocks.is_some() {
        sets.push("blocks = ?".to_string());
    }
    if meta_title.is_some() {
        sets.push("meta_title = ?".to_string());
    }
    if meta_description.is_some() {
        sets.push("meta_description = ?".to_string());
    }
    if og_image.is_some() {
        sets.push("og_image = ?".to_string());
    }
    if template.is_some() {
        sets.push("template = ?".to_string());
    }
    if parent_id.is_some() {
        sets.push("parent_id = ?".to_string());
    }
    if sort_order.is_some() {
        sets.push("sort_order = ?".to_string());
    }
    if status.is_some() {
        sets.push("status = ?".to_string());
        sets.push(
            "published_at = COALESCE(published_at, CASE WHEN ? = 'published' THEN ? ELSE NULL END)"
                .to_string(),
        );
    }
    if cover_image.is_some() {
        sets.push("cover_image = ?".to_string());
    }

    let tf = tenant_filter(tenant_id);
    let sql_str = format!("UPDATE pages SET {} WHERE id = ?{}", sets.join(", "), tf);
    let sql = crate::db::dialect::translate(&sql_str);

    let mut q = sqlx::query(&sql).bind(&now);
    if let Some(v) = updated_by {
        q = q.bind(v);
    }
    if let Some(v) = title {
        q = q.bind(v);
    }
    if let Some(v) = slug {
        q = q.bind(v);
    }
    if let Some(v) = content {
        q = q.bind(v);
    }
    if let Some(v) = blocks {
        q = q.bind(v);
    }
    if let Some(v) = meta_title {
        q = q.bind(v);
    }
    if let Some(v) = meta_description {
        q = q.bind(v);
    }
    if let Some(v) = og_image {
        q = q.bind(v);
    }
    if let Some(v) = template {
        q = q.bind(v);
    }
    if let Some(v) = parent_id {
        q = q.bind(v);
    }
    if let Some(v) = sort_order {
        q = q.bind(v);
    }
    if let Some(v) = status {
        q = q.bind(v);
        q = q.bind(v);
        q = q.bind(&now);
    }
    if let Some(v) = cover_image {
        q = q.bind(v);
    }
    q = q.bind(id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }

    let result = q.execute(pool).await?;
    AppError::expect_affected(&result, "page")?;

    find_by_id(pool, id, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found("page"))
}

pub async fn delete(pool: &crate::db::Pool, id: &str, tenant_id: Option<&str>) -> AppResult<()> {
    let sql = format!("DELETE FROM pages WHERE id = ?{}", tenant_filter(tenant_id));
    let sql = crate::db::dialect::translate(&sql);
    let mut q = sqlx::query(&sql).bind(id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    let result = q.execute(pool).await?;
    AppError::expect_affected(&result, "page")
}

pub async fn update_status(
    pool: &crate::db::Pool,
    id: &str,
    status: &str,
    updated_by: Option<&str>,
    tenant_id: Option<&str>,
) -> AppResult<Page> {
    let now = Utc::now().to_rfc3339();
    let published_at_clause = if status == "published" {
        "published_at = COALESCE(published_at, ?), ".to_string()
    } else {
        String::new()
    };
    let updated_by_clause = updated_by.map(|_| "updated_by = ?, ").unwrap_or("");
    let sql = format!(
        "UPDATE pages SET status = ?, {updated_by_clause}{published_at_clause}updated_at = ? WHERE id = ?{}",
        tenant_filter(tenant_id)
    );
    let sql = crate::db::dialect::translate(&sql);
    let mut q = sqlx::query(&sql).bind(status);
    if let Some(v) = updated_by {
        q = q.bind(v);
    }
    if status == "published" {
        q = q.bind(&now);
    }
    q = q.bind(&now).bind(id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    let result = q.execute(pool).await?;
    AppError::expect_affected(&result, "page")?;

    find_by_id(pool, id, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found("page"))
}

pub async fn reorder(
    pool: &crate::db::Pool,
    items: &[(String, i64)],
    tenant_id: Option<&str>,
) -> AppResult<()> {
    let now = Utc::now().to_rfc3339();
    let tf = tenant_filter(tenant_id);
    let sql_str = format!("UPDATE pages SET sort_order = ?, updated_at = ? WHERE id = ?{tf}");
    let sql = crate::db::dialect::translate(&sql_str);

    for (id, sort_order) in items {
        let mut q = sqlx::query(&sql).bind(sort_order).bind(&now).bind(id);
        if let Some(tid) = tenant_id {
            q = q.bind(tid);
        }
        q.execute(pool).await?;
    }
    Ok(())
}

pub async fn list_sitemap(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
) -> AppResult<Vec<(String, Option<String>)>> {
    let sql = format!(
        "SELECT slug, updated_at FROM pages WHERE status = 'published'{} ORDER BY sort_order ASC",
        tenant_filter(tenant_id)
    );
    let sql = crate::db::dialect::translate(&sql);
    let mut q = sqlx::query_as::<_, (String, Option<String>)>(&sql);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    Ok(q.fetch_all(pool).await?)
}

// ── 可复用块查询 ──

pub async fn find_reusable_by_id(
    pool: &crate::db::Pool,
    id: &str,
    tenant_id: Option<&str>,
) -> AppResult<Option<ReusableBlock>> {
    let sql = format!(
        "SELECT * FROM reusable_blocks WHERE id = ?{}",
        tenant_filter(tenant_id)
    );
    let sql = crate::db::dialect::translate(&sql);
    let mut q = sqlx::query_as::<_, ReusableBlock>(&sql).bind(id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    Ok(q.fetch_optional(pool).await?)
}

pub async fn list_reusable(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
) -> AppResult<Vec<ReusableBlock>> {
    let sql = format!(
        "SELECT * FROM reusable_blocks WHERE 1=1{} ORDER BY name ASC",
        tenant_filter(tenant_id)
    );
    let sql = crate::db::dialect::translate(&sql);
    let mut q = sqlx::query_as::<_, ReusableBlock>(&sql);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    Ok(q.fetch_all(pool).await?)
}

#[allow(clippy::too_many_arguments)]
pub async fn create_reusable(
    pool: &crate::db::Pool,
    id: &str,
    name: &str,
    block_type: &str,
    content: &str,
    description: Option<&str>,
    created_by: Option<&str>,
    tenant_id: Option<&str>,
) -> AppResult<ReusableBlock> {
    let now = Utc::now().to_rfc3339();
    match tenant_id {
        Some(tid) => {
            let sql = crate::db::dialect::translate(
                "INSERT INTO reusable_blocks (id, tenant_id, name, block_type, content, description, created_by, updated_by, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            );
            sqlx::query(&sql)
                .bind(id)
                .bind(tid)
                .bind(name)
                .bind(block_type)
                .bind(content)
                .bind(description)
                .bind(created_by)
                .bind(created_by)
                .bind(&now)
                .bind(&now)
                .execute(pool)
                .await?;
        }
        None => {
            let sql = crate::db::dialect::translate(
                "INSERT INTO reusable_blocks (id, name, block_type, content, description, created_by, updated_by, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            );
            sqlx::query(&sql)
                .bind(id)
                .bind(name)
                .bind(block_type)
                .bind(content)
                .bind(description)
                .bind(created_by)
                .bind(created_by)
                .bind(&now)
                .bind(&now)
                .execute(pool)
                .await?;
        }
    }

    find_reusable_by_id(pool, id, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found("reusable_block"))
}

#[allow(clippy::too_many_arguments)]
pub async fn update_reusable(
    pool: &crate::db::Pool,
    id: &str,
    name: Option<&str>,
    block_type: Option<&str>,
    content: Option<&str>,
    description: Option<&str>,
    updated_by: Option<&str>,
    tenant_id: Option<&str>,
) -> AppResult<ReusableBlock> {
    let now = Utc::now().to_rfc3339();
    let mut sets = vec!["updated_at = ?".to_string()];

    if updated_by.is_some() {
        sets.push("updated_by = ?".to_string());
    }
    if name.is_some() {
        sets.push("name = ?".to_string());
    }
    if block_type.is_some() {
        sets.push("block_type = ?".to_string());
    }
    if content.is_some() {
        sets.push("content = ?".to_string());
    }
    if description.is_some() {
        sets.push("description = ?".to_string());
    }

    let tf = tenant_filter(tenant_id);
    let sql_str = format!(
        "UPDATE reusable_blocks SET {} WHERE id = ?{}",
        sets.join(", "),
        tf
    );
    let sql = crate::db::dialect::translate(&sql_str);

    let mut q = sqlx::query(&sql).bind(&now);
    if let Some(v) = updated_by {
        q = q.bind(v);
    }
    if let Some(v) = name {
        q = q.bind(v);
    }
    if let Some(v) = block_type {
        q = q.bind(v);
    }
    if let Some(v) = content {
        q = q.bind(v);
    }
    if let Some(v) = description {
        q = q.bind(v);
    }
    q = q.bind(id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }

    let result = q.execute(pool).await?;
    AppError::expect_affected(&result, "reusable_block")?;

    find_reusable_by_id(pool, id, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found("reusable_block"))
}

pub async fn delete_reusable(
    pool: &crate::db::Pool,
    id: &str,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    let sql = format!(
        "DELETE FROM reusable_blocks WHERE id = ?{}",
        tenant_filter(tenant_id)
    );
    let sql = crate::db::dialect::translate(&sql);
    let mut q = sqlx::query(&sql).bind(id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    let result = q.execute(pool).await?;
    AppError::expect_affected(&result, "reusable_block")
}
