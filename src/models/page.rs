//! Page and block models with database queries

use serde::{Deserialize, Serialize};
#[cfg(feature = "export-types")]
use ts_rs::TS;

use crate::commands::{CreatePageCmd, UpdatePageCmd};

use crate::db::tenant::tenant_filter_ph;
use crate::db::{DbDriver, Driver};
use crate::errors::app_error::{AppError, AppResult};
use crate::models::post::CommentOpenStatus;
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::tz::Timestamp;

define_enum!(
    PageStatus {
        Draft = "draft",
        Published = "published",
    }
);

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct Page {
    pub id: SnowflakeId,
    pub tenant_id: Option<String>,
    pub title: String,
    pub slug: String,
    pub content: Option<String>,
    pub blocks: Option<String>,
    pub meta_title: Option<String>,
    pub meta_description: Option<String>,
    pub og_image: Option<String>,
    pub template: String,
    pub parent_id: Option<SnowflakeId>,
    pub sort_order: i64,
    pub status: PageStatus,
    pub created_by: SnowflakeId,
    pub updated_by: Option<SnowflakeId>,
    pub cover_image: Option<String>,
    pub password: Option<String>,
    pub comment_status: CommentOpenStatus,
    pub og_title: Option<String>,
    pub og_description: Option<String>,
    pub canonical_url: Option<String>,
    pub published_at: Option<Timestamp>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

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
    Ok(raisfast_derive::crud_find!(pool, "pages", Page, where: ("slug", slug), tenant: tenant_id)?)
}

pub async fn find_by_id(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    tenant_id: Option<&str>,
) -> AppResult<Option<Page>> {
    Ok(raisfast_derive::crud_find!(pool, "pages", Page, where: ("id", id), tenant: tenant_id)?)
}

pub async fn list_published(
    pool: &crate::db::Pool,
    page: i64,
    page_size: i64,
    tenant_id: Option<&str>,
) -> AppResult<(Vec<Page>, i64)> {
    let result = raisfast_derive::crud_query_paged!(
        pool, Page,
        table: "pages",
        where: ("status", PageStatus::Published),
        order_by: "sort_order ASC, created_at DESC",
        tenant: tenant_id,
        page: page,
        page_size: page_size
    );
    Ok(result)
}

pub async fn list_all(
    pool: &crate::db::Pool,
    page: i64,
    page_size: i64,
    status: Option<PageStatus>,
    tenant_id: Option<&str>,
) -> AppResult<(Vec<Page>, i64)> {
    let result = raisfast_derive::crud_query_paged!(
        pool, Page,
        table: "pages",
        where: ["status" => status],
        order_by: "sort_order ASC, created_at DESC",
        tenant: tenant_id,
        page: page,
        page_size: page_size
    );
    Ok(result)
}

pub async fn create(
    pool: &crate::db::Pool,
    cmd: &CreatePageCmd,
    tenant_id: Option<&str>,
) -> AppResult<Page> {
    let (id, now) = (
        crate::utils::id::new_snowflake_id(),
        crate::utils::tz::now_utc(),
    );
    let published_at = if cmd.status == PageStatus::Published {
        Some(now)
    } else {
        None
    };

    raisfast_derive::crud_insert!(
        pool,
        "pages",
        [
            "id" => id,
            "title" => &cmd.title,
            "slug" => &cmd.slug,
            "content" => &cmd.content,
            "blocks" => &cmd.blocks,
            "meta_title" => &cmd.meta_title,
            "meta_description" => &cmd.meta_description,
            "og_image" => &cmd.og_image,
            "template" => &cmd.template,
            "parent_id" => cmd.parent_id,
            "sort_order" => cmd.sort_order,
            "status" => cmd.status,
            "created_by" => cmd.created_by,
            "updated_by" => cmd.created_by,
            "cover_image" => &cmd.cover_image,
            "published_at" => published_at,
            "created_at" => now,
            "updated_at" => now
        ],
        tenant: tenant_id
    )?;

    find_by_id(pool, id, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found("page"))
}

pub async fn update(
    pool: &crate::db::Pool,
    cmd: &UpdatePageCmd,
    tenant_id: Option<&str>,
) -> AppResult<Page> {
    raisfast_derive::check_schema!(
        "pages",
        "updated_by",
        "title",
        "slug",
        "content",
        "blocks",
        "meta_title",
        "meta_description",
        "og_image",
        "template",
        "parent_id",
        "sort_order",
        "status",
        "published_at",
        "cover_image",
        "updated_at",
        "id"
    );
    let now = crate::utils::tz::now_utc();
    let mut idx = 0;
    let mut sets = vec![];

    if cmd.updated_by.is_some() {
        idx += 1;
        sets.push(format!("updated_by = {}", Driver::ph(idx)));
    }
    if cmd.title.is_some() {
        idx += 1;
        sets.push(format!("title = {}", Driver::ph(idx)));
    }
    if cmd.slug.is_some() {
        idx += 1;
        sets.push(format!("slug = {}", Driver::ph(idx)));
    }
    if cmd.content.is_some() {
        idx += 1;
        sets.push(format!("content = {}", Driver::ph(idx)));
    }
    if cmd.blocks.is_some() {
        idx += 1;
        sets.push(format!("blocks = {}", Driver::ph(idx)));
    }
    if cmd.meta_title.is_some() {
        idx += 1;
        sets.push(format!("meta_title = {}", Driver::ph(idx)));
    }
    if cmd.meta_description.is_some() {
        idx += 1;
        sets.push(format!("meta_description = {}", Driver::ph(idx)));
    }
    if cmd.og_image.is_some() {
        idx += 1;
        sets.push(format!("og_image = {}", Driver::ph(idx)));
    }
    if cmd.template.is_some() {
        idx += 1;
        sets.push(format!("template = {}", Driver::ph(idx)));
    }
    if cmd.parent_id.is_some() {
        idx += 1;
        sets.push(format!("parent_id = {}", Driver::ph(idx)));
    }
    if cmd.sort_order.is_some() {
        idx += 1;
        sets.push(format!("sort_order = {}", Driver::ph(idx)));
    }
    if cmd.status.is_some() {
        idx += 1;
        sets.push(format!("status = {}", Driver::ph(idx)));
        idx += 1;
        let s1 = Driver::ph(idx);
        idx += 1;
        let s2 = Driver::ph(idx);
        sets.push(format!(
            "published_at = COALESCE(published_at, CASE WHEN {s1} = 'published' THEN {s2} ELSE NULL END)"
        ));
    }
    if cmd.cover_image.is_some() {
        idx += 1;
        sets.push(format!("cover_image = {}", Driver::ph(idx)));
    }

    idx += 1;
    sets.push(format!("updated_at = {}", Driver::ph(idx)));

    idx += 1;
    let id_ph = Driver::ph(idx);
    let tf = tenant_filter_ph(tenant_id, idx + 1);
    let sql = format!(
        "UPDATE pages SET {} WHERE id = {id_ph}{tf}",
        sets.join(", ")
    );

    let mut q = sqlx::query(&sql);
    if let Some(v) = cmd.updated_by {
        q = q.bind(v);
    }
    if let Some(ref v) = cmd.title {
        q = q.bind(v);
    }
    if let Some(ref v) = cmd.slug {
        q = q.bind(v);
    }
    if let Some(ref v) = cmd.content {
        q = q.bind(v);
    }
    if let Some(ref v) = cmd.blocks {
        q = q.bind(v);
    }
    if let Some(ref v) = cmd.meta_title {
        q = q.bind(v);
    }
    if let Some(ref v) = cmd.meta_description {
        q = q.bind(v);
    }
    if let Some(ref v) = cmd.og_image {
        q = q.bind(v);
    }
    if let Some(ref v) = cmd.template {
        q = q.bind(v);
    }
    if let Some(v) = cmd.parent_id {
        q = q.bind(v);
    }
    if let Some(v) = cmd.sort_order {
        q = q.bind(v);
    }
    if let Some(v) = cmd.status {
        q = q.bind(v);
        q = q.bind(v);
        q = q.bind(now);
    }
    if let Some(ref v) = cmd.cover_image {
        q = q.bind(v);
    }
    q = q.bind(now);
    q = q.bind(cmd.id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }

    let result = q.execute(pool).await?;
    AppError::expect_affected(&result, "page")?;

    find_by_id(pool, cmd.id, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found("page"))
}

pub async fn delete(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    let result =
        raisfast_derive::crud_delete!(pool, "pages", where: ("id", id), tenant: tenant_id)?;
    AppError::expect_affected(&result, "page")
}

pub async fn update_status(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    status: PageStatus,
    updated_by: Option<i64>,
    tenant_id: Option<&str>,
) -> AppResult<Page> {
    raisfast_derive::check_schema!(
        "pages",
        "status",
        "updated_by",
        "published_at",
        "updated_at",
        "id"
    );
    let now = crate::utils::tz::now_utc();
    let mut idx = 1;
    let status_ph = Driver::ph(idx);

    let updated_by_clause = if updated_by.is_some() {
        idx += 1;
        format!(", updated_by = {}", Driver::ph(idx))
    } else {
        String::new()
    };

    let published_at_clause = if status == PageStatus::Published {
        idx += 1;
        format!(
            ", published_at = COALESCE(published_at, {})",
            Driver::ph(idx)
        )
    } else {
        String::new()
    };

    idx += 1;
    let updated_at_clause = format!(", updated_at = {}", Driver::ph(idx));

    idx += 1;
    let id_ph = Driver::ph(idx);
    let tf = tenant_filter_ph(tenant_id, idx + 1);

    let sql = format!(
        "UPDATE pages SET status = {status_ph}{updated_by_clause}{published_at_clause}{updated_at_clause} WHERE id = {id_ph}{tf}"
    );
    let mut q = sqlx::query(&sql).bind(status);
    if let Some(v) = updated_by {
        q = q.bind(v);
    }
    if status == PageStatus::Published {
        q = q.bind(now);
    }
    q = q.bind(now);
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

pub async fn reorder(
    pool: &crate::db::Pool,
    items: &[(i64, i64)],
    tenant_id: Option<&str>,
) -> AppResult<()> {
    let now = crate::utils::tz::now_utc();

    for (id, sort_order) in items {
        raisfast_derive::crud_update!(
            pool, "pages",
            bind: ["sort_order" => sort_order, "updated_at" => now],
            where: ("id", id),
            tenant: tenant_id
        )?;
    }
    Ok(())
}

pub async fn list_sitemap(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
) -> AppResult<Vec<(String, Option<String>)>> {
    let filter = tenant_filter_ph(tenant_id, 2);
    let sql = format!(
        "SELECT slug, updated_at FROM pages WHERE status = {}{filter} ORDER BY sort_order ASC",
        Driver::ph(1)
    );
    Ok(raisfast_derive::crud_query!(
        pool,
        (String, Option<String>),
        &sql,
        [PageStatus::Published],
        fetch_all,
        tenant: tenant_id
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn setup_pool() -> crate::db::Pool {
        crate::test_pool!()
    }

    async fn create_user(pool: &crate::db::Pool) -> i64 {
        let id = crate::utils::id::new_id();
        sqlx::query(
            "INSERT INTO users (id, username, role, status, registered_via) VALUES (?, 'testuser', 'author', 'active', 'email')",
        )
        .bind(id)
        .execute(pool)
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
            pool,
            &CreatePageCmd {
                title: title.to_string(),
                slug: slug.to_string(),
                content: None,
                blocks: None,
                meta_title: None,
                meta_description: None,
                og_image: None,
                template: "default".to_string(),
                parent_id: None,
                sort_order: 0,
                status: status.parse().unwrap(),
                created_by,
                updated_by: None,
                cover_image: None,
            },
            None,
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
            &UpdatePageCmd {
                id: page.id,
                title: Some("New Title".to_string()),
                slug: None,
                content: None,
                blocks: None,
                meta_title: None,
                meta_description: None,
                og_image: None,
                template: None,
                parent_id: None,
                sort_order: None,
                status: None,
                cover_image: None,
                updated_by: None,
            },
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

        assert_eq!(page.status, PageStatus::Draft);

        let updated = update_status(&pool, page.id, PageStatus::Published, None, None)
            .await
            .unwrap();
        assert_eq!(updated.status, PageStatus::Published);
        assert!(updated.published_at.is_some());
    }
}
