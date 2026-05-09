//! 页面与块模型及数据库查询

use serde::{Deserialize, Serialize};
#[cfg(feature = "export-types")]
use ts_rs::TS;

use crate::db::dialect::ph;
use crate::db::tenant::tenant_filter_ph;
use crate::errors::app_error::{AppError, AppResult};
use crate::utils::tz::Timestamp;

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Page {
    pub id: i64,
    pub document_id: String,
    pub tenant_id: Option<String>,
    pub title: String,
    pub slug: String,
    pub content: Option<String>,
    pub blocks: Option<String>,
    pub meta_title: Option<String>,
    pub meta_description: Option<String>,
    pub og_image: Option<String>,
    pub template: String,
    pub parent_id: Option<i64>,
    pub sort_order: i64,
    pub status: String,
    pub created_by: i64,
    pub updated_by: Option<i64>,
    pub cover_image: Option<String>,
    pub published_at: Option<Timestamp>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

crate::impl_from_row_opt_tenant!(Page {
    required { id, document_id, title, slug, template, sort_order, status, created_by, created_at, updated_at }
    optional { content, blocks, meta_title, meta_description, og_image, parent_id, updated_by, cover_image, published_at }
});

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

pub async fn find_by_slug(
    pool: &crate::db::Pool,
    slug: &str,
    tenant_id: Option<&str>,
) -> AppResult<Option<Page>> {
    let filter = tenant_filter_ph(tenant_id, 2);
    let sql = format!("SELECT * FROM pages WHERE slug = {}{filter}", ph(1));
    let mut q = sqlx::query_as::<_, Page>(&sql).bind(slug);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    Ok(q.fetch_optional(pool).await?)
}

pub async fn find_by_id(
    pool: &crate::db::Pool,
    id: i64,
    tenant_id: Option<&str>,
) -> AppResult<Option<Page>> {
    let filter = tenant_filter_ph(tenant_id, 2);
    let sql = format!("SELECT * FROM pages WHERE id = {}{filter}", ph(1));
    let mut q = sqlx::query_as::<_, Page>(&sql).bind(id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    Ok(q.fetch_optional(pool).await?)
}

pub async fn find_by_document_id(
    pool: &crate::db::Pool,
    document_id: &str,
    tenant_id: Option<&str>,
) -> AppResult<Option<Page>> {
    let filter = tenant_filter_ph(tenant_id, 2);
    let sql = format!("SELECT * FROM pages WHERE document_id = {}{filter}", ph(1));
    let mut q = sqlx::query_as::<_, Page>(&sql).bind(document_id);
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
    let count_filter = tenant_filter_ph(tenant_id, 1);
    let count_sql = format!("SELECT COUNT(*) FROM pages WHERE status = 'published'{count_filter}");
    let mut cq = sqlx::query_scalar::<_, i64>(&count_sql);
    if let Some(tid) = tenant_id {
        cq = cq.bind(tid);
    }
    let total = cq.fetch_one(pool).await?;

    let base = usize::from(tenant_id.is_some());
    let data_filter = tenant_filter_ph(tenant_id, 1);
    let data_sql = format!(
        "SELECT * FROM pages WHERE status = 'published'{data_filter} ORDER BY sort_order ASC, created_at DESC LIMIT {} OFFSET {}",
        ph(base + 1),
        ph(base + 2)
    );
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
    let has_status = status.is_some();
    let status_clause = if has_status {
        format!(" AND status = {}", ph(1))
    } else {
        String::new()
    };

    let tf = tenant_filter_ph(tenant_id, has_status as usize + 1);
    let count_sql = format!("SELECT COUNT(*) FROM pages WHERE 1=1{status_clause}{tf}");
    let mut cq = sqlx::query_scalar::<_, i64>(&count_sql);
    if let Some(s) = status {
        cq = cq.bind(s);
    }
    if let Some(tid) = tenant_id {
        cq = cq.bind(tid);
    }
    let total = cq.fetch_one(pool).await?;

    let base = has_status as usize + usize::from(tenant_id.is_some());
    let data_sql = format!(
        "SELECT * FROM pages WHERE 1=1{status_clause}{tf} ORDER BY sort_order ASC, created_at DESC LIMIT {} OFFSET {}",
        ph(base + 1),
        ph(base + 2)
    );
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
    title: &str,
    slug: &str,
    content: Option<&str>,
    blocks: Option<&str>,
    meta_title: Option<&str>,
    meta_description: Option<&str>,
    og_image: Option<&str>,
    template: &str,
    parent_id: Option<i64>,
    sort_order: i64,
    status: &str,
    created_by: i64,
    cover_image: Option<&str>,
    tenant_id: Option<&str>,
) -> AppResult<Page> {
    let (document_id, now) = crate::utils::id::new_document_id_and_timestamp();
    let published_at = if status == "published" {
        Some(now)
    } else {
        None
    };

    match tenant_id {
        Some(tid) => {
            let vals = (1..=19).map(ph).collect::<Vec<_>>().join(", ");
            let sql = format!(
                "INSERT INTO pages (document_id, tenant_id, title, slug, content, blocks, meta_title, meta_description, og_image, template, parent_id, sort_order, status, created_by, updated_by, cover_image, published_at, created_at, updated_at) VALUES ({vals})"
            );
            sqlx::query(&sql)
                .bind(&document_id)
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
                .bind(published_at)
                .bind(now)
                .bind(now)
                .execute(pool)
                .await?;
        }
        None => {
            let vals = (1..=18).map(ph).collect::<Vec<_>>().join(", ");
            let sql = format!(
                "INSERT INTO pages (document_id, title, slug, content, blocks, meta_title, meta_description, og_image, template, parent_id, sort_order, status, created_by, updated_by, cover_image, published_at, created_at, updated_at) VALUES ({vals})"
            );
            sqlx::query(&sql)
                .bind(&document_id)
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
                .bind(published_at)
                .bind(now)
                .bind(now)
                .execute(pool)
                .await?;
        }
    }

    find_by_document_id(pool, &document_id, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found("page"))
}

#[allow(clippy::too_many_arguments)]
pub async fn update(
    pool: &crate::db::Pool,
    id: i64,
    title: Option<&str>,
    slug: Option<&str>,
    content: Option<&str>,
    blocks: Option<&str>,
    meta_title: Option<&str>,
    meta_description: Option<&str>,
    og_image: Option<&str>,
    template: Option<&str>,
    parent_id: Option<Option<i64>>,
    sort_order: Option<i64>,
    status: Option<&str>,
    cover_image: Option<&str>,
    updated_by: Option<i64>,
    tenant_id: Option<&str>,
) -> AppResult<Page> {
    let now = crate::utils::tz::now_utc();
    let mut idx = 1;
    let mut sets = vec![format!("updated_at = {}", ph(idx))];

    if updated_by.is_some() {
        idx += 1;
        sets.push(format!("updated_by = {}", ph(idx)));
    }
    if title.is_some() {
        idx += 1;
        sets.push(format!("title = {}", ph(idx)));
    }
    if slug.is_some() {
        idx += 1;
        sets.push(format!("slug = {}", ph(idx)));
    }
    if content.is_some() {
        idx += 1;
        sets.push(format!("content = {}", ph(idx)));
    }
    if blocks.is_some() {
        idx += 1;
        sets.push(format!("blocks = {}", ph(idx)));
    }
    if meta_title.is_some() {
        idx += 1;
        sets.push(format!("meta_title = {}", ph(idx)));
    }
    if meta_description.is_some() {
        idx += 1;
        sets.push(format!("meta_description = {}", ph(idx)));
    }
    if og_image.is_some() {
        idx += 1;
        sets.push(format!("og_image = {}", ph(idx)));
    }
    if template.is_some() {
        idx += 1;
        sets.push(format!("template = {}", ph(idx)));
    }
    if parent_id.is_some() {
        idx += 1;
        sets.push(format!("parent_id = {}", ph(idx)));
    }
    if sort_order.is_some() {
        idx += 1;
        sets.push(format!("sort_order = {}", ph(idx)));
    }
    if status.is_some() {
        idx += 1;
        sets.push(format!("status = {}", ph(idx)));
        idx += 1;
        let s1 = ph(idx);
        idx += 1;
        let s2 = ph(idx);
        sets.push(format!(
            "published_at = COALESCE(published_at, CASE WHEN {s1} = 'published' THEN {s2} ELSE NULL END)"
        ));
    }
    if cover_image.is_some() {
        idx += 1;
        sets.push(format!("cover_image = {}", ph(idx)));
    }

    idx += 1;
    let id_ph = ph(idx);
    let tf = tenant_filter_ph(tenant_id, idx + 1);
    let sql = format!(
        "UPDATE pages SET {} WHERE id = {id_ph}{tf}",
        sets.join(", ")
    );

    let mut q = sqlx::query(&sql).bind(now);
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
        q = q.bind(now);
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

pub async fn delete(pool: &crate::db::Pool, id: i64, tenant_id: Option<&str>) -> AppResult<()> {
    let filter = tenant_filter_ph(tenant_id, 2);
    let sql = format!("DELETE FROM pages WHERE id = {}{filter}", ph(1));
    let mut q = sqlx::query(&sql).bind(id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    let result = q.execute(pool).await?;
    AppError::expect_affected(&result, "page")
}

pub async fn update_status(
    pool: &crate::db::Pool,
    id: i64,
    status: &str,
    updated_by: Option<i64>,
    tenant_id: Option<&str>,
) -> AppResult<Page> {
    let now = crate::utils::tz::now_utc();
    let mut idx = 1;
    let status_ph = ph(idx);

    let updated_by_clause = if updated_by.is_some() {
        idx += 1;
        format!("updated_by = {}, ", ph(idx))
    } else {
        String::new()
    };

    let published_at_clause = if status == "published" {
        idx += 1;
        format!("published_at = COALESCE(published_at, {}), ", ph(idx))
    } else {
        String::new()
    };

    idx += 1;
    let updated_at_ph = ph(idx);
    idx += 1;
    let id_ph = ph(idx);
    let tf = tenant_filter_ph(tenant_id, idx + 1);

    let sql = format!(
        "UPDATE pages SET status = {status_ph}, {updated_by_clause}{published_at_clause}updated_at = {updated_at_ph} WHERE id = {id_ph}{tf}"
    );
    let mut q = sqlx::query(&sql).bind(status);
    if let Some(v) = updated_by {
        q = q.bind(v);
    }
    if status == "published" {
        q = q.bind(now);
    }
    q = q.bind(now).bind(id);
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
    items: &[(i64, i64)],
    tenant_id: Option<&str>,
) -> AppResult<()> {
    let now = crate::utils::tz::now_utc();
    let tf = tenant_filter_ph(tenant_id, 4);
    let sql = format!(
        "UPDATE pages SET sort_order = {}, updated_at = {} WHERE id = {}{tf}",
        ph(1),
        ph(2),
        ph(3)
    );

    for (id, sort_order) in items {
        let mut q = sqlx::query(&sql).bind(sort_order).bind(now).bind(id);
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
    let filter = tenant_filter_ph(tenant_id, 1);
    let sql = format!(
        "SELECT slug, updated_at FROM pages WHERE status = 'published'{filter} ORDER BY sort_order ASC"
    );
    let mut q = sqlx::query_as::<_, (String, Option<String>)>(&sql);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    Ok(q.fetch_all(pool).await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn setup_pool() -> crate::db::Pool {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        pool
    }

    async fn create_user(pool: &crate::db::Pool) -> i64 {
        let uid = uuid::Uuid::now_v7().to_string();
        sqlx::query(
            "INSERT INTO users (document_id, username, email, password_hash, role) VALUES (?, 'testuser', 'test@test.com', 'hash', 'author')",
        )
        .bind(&uid)
        .execute(pool)
        .await
        .unwrap();

        let (id,): (i64,) = sqlx::query_as("SELECT id FROM users WHERE document_id = ?")
            .bind(&uid)
            .fetch_one(pool)
            .await
            .unwrap();
        id
    }

    async fn create_test_page(
        pool: &crate::db::Pool,
        title: &str,
        slug: &str,
        status: &str,
        created_by: i64,
    ) -> Page {
        create(
            pool, title, slug, None, None, None, None, None, "default", None, 0, status,
            created_by, None, None,
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn create_and_find_by_slug() {
        let pool = setup_pool().await;
        let uid = create_user(&pool).await;
        let page = create_test_page(&pool, "About Us", "about", "published", uid).await;

        let found = find_by_slug(&pool, "about", None).await.unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, page.id);
    }

    #[tokio::test]
    async fn find_by_document_id_test() {
        let pool = setup_pool().await;
        let uid = create_user(&pool).await;
        let page = create_test_page(&pool, "Contact", "contact", "draft", uid).await;

        let found = super::find_by_document_id(&pool, &page.document_id, None)
            .await
            .unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().slug, "contact");
    }

    #[tokio::test]
    async fn list_published_excludes_drafts() {
        let pool = setup_pool().await;
        let uid = create_user(&pool).await;
        create_test_page(&pool, "Published Page", "pub", "published", uid).await;
        create_test_page(&pool, "Draft Page", "draft", "draft", uid).await;

        let (items, total) = list_published(&pool, 1, 10, None).await.unwrap();
        assert_eq!(total, 1);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].slug, "pub");
    }

    #[tokio::test]
    async fn update_changes_title() {
        let pool = setup_pool().await;
        let uid = create_user(&pool).await;
        let page = create_test_page(&pool, "Old Title", "old", "published", uid).await;

        let updated = update(
            &pool,
            page.id,
            Some("New Title"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

        assert_eq!(updated.title, "New Title");
    }

    #[tokio::test]
    async fn delete_removes_page() {
        let pool = setup_pool().await;
        let uid = create_user(&pool).await;
        let page = create_test_page(&pool, "To Delete", "delete-me", "published", uid).await;

        delete(&pool, page.id, None).await.unwrap();
        let found = find_by_slug(&pool, "delete-me", None).await.unwrap();
        assert!(found.is_none());
    }

    #[tokio::test]
    async fn update_status_changes() {
        let pool = setup_pool().await;
        let uid = create_user(&pool).await;
        let page = create_test_page(&pool, "Status Test", "status-test", "draft", uid).await;

        assert_eq!(page.status, "draft");

        let updated = update_status(&pool, page.id, "published", None, None)
            .await
            .unwrap();
        assert_eq!(updated.status, "published");
        assert!(updated.published_at.is_some());
    }
}
