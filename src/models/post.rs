//! Post model and database queries
//!
//! Defines data structures related to posts, including the full row model,
//! frontend-facing response models, tag summary structs, and all CRUD operations
//! on the `posts` table and associated tables.
//!
//! Also provides helper query functions for fetching author names, category names,
//! post tags, and other related data, as well as paginated queries for published
//! posts filtered by category, tag, or keyword.

use serde::{Deserialize, Serialize};
use sqlx::FromRow;
#[cfg(feature = "export-types")]
use ts_rs::TS;

use crate::db::tenant::{tenant_filter_aliased_ph, tenant_filter_ph};
use crate::db::{DbDriver, Driver};
use crate::errors::app_error::{AppError, AppResult};
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::tz::Timestamp;

define_enum!(
    PostStatus {
        Draft = "draft",
        Published = "published",
    }
);

define_enum!(
    CommentOpenStatus {
        Open = "open",
        Closed = "closed",
    }
);

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
#[non_exhaustive]
pub struct Post {
    pub id: SnowflakeId,
    pub tenant_id: Option<String>,
    pub title: String,
    pub slug: String,
    pub content: String,
    pub excerpt: Option<String>,
    pub cover_image: Option<String>,
    pub image_ids: Option<String>,
    pub status: PostStatus,
    pub created_by: SnowflakeId,
    pub updated_by: Option<SnowflakeId>,
    pub category_id: Option<SnowflakeId>,
    pub view_count: i64,
    pub is_pinned: bool,
    pub password: Option<String>,
    pub comment_status: CommentOpenStatus,
    pub format: String,
    pub template: String,
    pub meta_title: Option<String>,
    pub meta_description: Option<String>,
    pub og_title: Option<String>,
    pub og_description: Option<String>,
    pub og_image: Option<String>,
    pub canonical_url: Option<String>,
    pub reading_time: i64,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub published_at: Option<Timestamp>,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, Deserialize, Clone, utoipa::ToSchema)]
pub struct TagBrief {
    pub id: String,
    pub name: String,
    pub slug: String,
}

pub async fn find_by_slug(
    pool: &crate::db::Pool,
    slug: &str,
    tenant_id: Option<&str>,
) -> AppResult<Option<Post>> {
    let post =
        raisfast_derive::crud_find!(pool, "posts", Post, where: ("slug", slug), tenant: tenant_id)?;
    Ok(post)
}

pub async fn find_by_id(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    tenant_id: Option<&str>,
) -> AppResult<Option<Post>> {
    let post =
        raisfast_derive::crud_find!(pool, "posts", Post, where: ("id", id), tenant: tenant_id)?;
    Ok(post)
}

pub async fn create(
    pool: &crate::db::Pool,
    cmd: &crate::commands::CreatePostCmd,
    tenant_id: Option<&str>,
) -> AppResult<Post> {
    let _guard = crate::db::connection::acquire_write().await;
    let mut tx = pool.begin().await?;
    let post = create_tx(&mut tx, cmd, tenant_id).await?;
    tx.commit().await?;
    Ok(post)
}

pub async fn create_tx(
    tx: &mut crate::db::Transaction<'_>,
    cmd: &crate::commands::CreatePostCmd,
    tenant_id: Option<&str>,
) -> AppResult<Post> {
    let (id, now) = (
        crate::utils::id::new_snowflake_id(),
        crate::utils::tz::now_utc(),
    );
    let published_at = if cmd.status == PostStatus::Published {
        Some(now)
    } else {
        None
    };
    raisfast_derive::crud_insert!(
        &mut **tx,
        "posts",
        [
            "id" => id,
            "title" => &cmd.title,
            "slug" => &cmd.slug,
            "content" => &cmd.content,
            "excerpt" => &cmd.excerpt,
            "cover_image" => &cmd.cover_image,
            "image_ids" => &cmd.image_ids,
            "status" => cmd.status,
            "created_by" => cmd.created_by,
            "updated_by" => cmd.updated_by,
            "category_id" => cmd.category_id,
            "meta_title" => &cmd.meta_title,
            "meta_description" => &cmd.meta_description,
            "og_title" => &cmd.og_title,
            "og_description" => &cmd.og_description,
            "og_image" => &cmd.og_image,
            "canonical_url" => &cmd.canonical_url,
            "published_at" => published_at,
            "created_at" => now,
            "updated_at" => now
        ],
        tenant: tenant_id
    )?;

    let created = find_by_id_tx(tx, id, tenant_id)
        .await?
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("failed to read created post")))?;

    Ok(created)
}

async fn find_by_id_tx(
    tx: &mut crate::db::Transaction<'_>,
    id: SnowflakeId,
    tenant_id: Option<&str>,
) -> AppResult<Option<Post>> {
    raisfast_derive::crud_find!(&mut **tx, "posts", Post, where: ("id", id), tenant: tenant_id)
        .map_err(Into::into)
}

pub async fn update(
    pool: &crate::db::Pool,
    cmd: &crate::commands::UpdatePostCmd,
    tenant_id: Option<&str>,
) -> AppResult<Post> {
    let _guard = crate::db::connection::acquire_write().await;
    let mut tx = pool.begin().await?;
    let post = update_tx(&mut tx, cmd, tenant_id).await?;
    tx.commit().await?;
    Ok(post)
}

pub async fn update_tx(
    tx: &mut crate::db::Transaction<'_>,
    cmd: &crate::commands::UpdatePostCmd,
    tenant_id: Option<&str>,
) -> AppResult<Post> {
    let post_id = cmd.id;
    let existing =
        raisfast_derive::crud_find_one!(&mut **tx, "posts", Post, where: ("id", post_id), tenant: tenant_id)
            .map_err(|_| AppError::not_found("post"))?;

    let now = crate::utils::tz::now_utc();
    let new_status = match cmd.status {
        Some(ref s) => *s,
        None => existing.status,
    };
    let published_at = if new_status == PostStatus::Published && existing.published_at.is_none() {
        Some(now)
    } else {
        existing.published_at
    };

    let title = cmd.title.as_deref().unwrap_or(&existing.title);
    let content = cmd.content.as_deref().unwrap_or(&existing.content);
    let excerpt = cmd
        .excerpt
        .as_deref()
        .map(std::string::ToString::to_string)
        .or(existing.excerpt);
    let cover_image = cmd
        .cover_image
        .as_deref()
        .map(std::string::ToString::to_string)
        .or(existing.cover_image);
    let image_ids = cmd
        .image_ids
        .as_deref()
        .map(std::string::ToString::to_string)
        .or(existing.image_ids);
    let category_id: Option<i64> = cmd.category_id.or(existing.category_id.map(|v| *v));
    let slug = cmd.slug.as_deref().unwrap_or(&existing.slug);
    let updated_by: Option<i64> = cmd.updated_by.or(existing.updated_by.map(|v| *v));
    let meta_title = cmd
        .meta_title
        .as_deref()
        .map(std::string::ToString::to_string)
        .or(existing.meta_title);
    let meta_description = cmd
        .meta_description
        .as_deref()
        .map(std::string::ToString::to_string)
        .or(existing.meta_description);
    let og_title = cmd
        .og_title
        .as_deref()
        .map(std::string::ToString::to_string)
        .or(existing.og_title);
    let og_description = cmd
        .og_description
        .as_deref()
        .map(std::string::ToString::to_string)
        .or(existing.og_description);
    let og_image = cmd
        .og_image
        .as_deref()
        .map(std::string::ToString::to_string)
        .or(existing.og_image);
    let canonical_url = cmd
        .canonical_url
        .as_deref()
        .map(std::string::ToString::to_string)
        .or(existing.canonical_url);

    raisfast_derive::crud_update!(
        &mut **tx, "posts",
        bind: [
            "title" => title, "slug" => slug, "content" => content,
            "excerpt" => &excerpt, "cover_image" => &cover_image,
            "image_ids" => &image_ids,
            "status" => new_status, "category_id" => category_id,
            "published_at" => published_at, "updated_by" => updated_by,
            "meta_title" => &meta_title, "meta_description" => &meta_description,
            "og_title" => &og_title, "og_description" => &og_description,
            "og_image" => &og_image, "canonical_url" => &canonical_url,
            "updated_at" => now
        ],
        where: ("id", post_id),
        tenant: tenant_id
    )?;

    Ok(Post {
        id: existing.id,
        tenant_id: existing.tenant_id,
        title: title.to_string(),
        slug: slug.to_string(),
        content: content.to_string(),
        excerpt,
        cover_image,
        image_ids,
        status: new_status,
        created_by: existing.created_by,
        updated_by: updated_by.map(SnowflakeId),
        category_id: category_id.map(SnowflakeId),
        view_count: existing.view_count,
        is_pinned: existing.is_pinned,
        password: existing.password,
        comment_status: existing.comment_status,
        format: existing.format,
        template: existing.template,
        meta_title,
        meta_description,
        og_title,
        og_description,
        og_image,
        canonical_url,
        reading_time: existing.reading_time,
        created_at: existing.created_at,
        updated_at: now,
        published_at,
    })
}

pub async fn delete(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    let result =
        raisfast_derive::crud_delete!(pool, "posts", where: ("id", id), tenant: tenant_id)?;
    AppError::expect_affected(&result, "post")
}

pub async fn increment_view_count_joined(
    pool: &crate::db::Pool,
    slug: &str,
    tenant_id: Option<&str>,
) -> AppResult<PostJoinedRow> {
    raisfast_derive::crud_update!(
        pool, "posts",
        raw: ["view_count" => "view_count + 1"],
        where: AND(("slug", slug), ("status", PostStatus::Published)),
        tenant: tenant_id
    )?;

    find_published_joined_by_slug(pool, slug, tenant_id).await
}

#[derive(Debug, FromRow)]
pub struct TagRow {
    pub id: SnowflakeId,
    pub name: String,
    pub slug: String,
}

pub async fn get_post_tags(
    pool: &crate::db::Pool,
    post_id: SnowflakeId,
    _tenant_id: Option<&str>,
) -> AppResult<Vec<TagBrief>> {
    crate::models::tagging::get_tags_for(pool, "post", post_id).await
}

pub async fn get_tags_by_ids(
    pool: &crate::db::Pool,
    tag_ids: &[i64],
    tenant_id: Option<&str>,
) -> AppResult<Vec<TagBrief>> {
    if tag_ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows: Vec<TagRow> = raisfast_derive::crud_find_all!(
        pool, "tags", TagRow,
        where: ("id", IN, tag_ids),
        tenant: tenant_id
    )?;
    Ok(rows
        .into_iter()
        .map(|r| TagBrief {
            id: r.id.to_string(),
            name: r.name,
            slug: r.slug,
        })
        .collect())
}

pub async fn get_author_name(
    pool: &crate::db::Pool,
    created_by: i64,
    tenant_id: Option<&str>,
) -> AppResult<Option<String>> {
    let row: Option<(String,)> = raisfast_derive::crud_select!(
        pool, "users", ["username"], where: ("id", created_by), tenant: tenant_id
    )?;
    Ok(row.map(|(s,)| s))
}

pub async fn get_category_name(
    pool: &crate::db::Pool,
    category_id: SnowflakeId,
    tenant_id: Option<&str>,
) -> AppResult<Option<String>> {
    let row: Option<(String,)> = raisfast_derive::crud_select!(
        pool, "categories", ["name"], where: ("id", category_id), tenant: tenant_id
    )?;
    Ok(row.map(|(s,)| s))
}

pub async fn find_published(
    pool: &crate::db::Pool,
    page: i64,
    page_size: i64,
    category_id: Option<i64>,
    tag_id: Option<i64>,
    q: Option<&str>,
    tenant_id: Option<&str>,
) -> AppResult<(Vec<Post>, i64)> {
    raisfast_derive::check_schema!(
        "posts",
        "id",
        "status",
        "is_pinned",
        "created_at",
        "title",
        "content",
        "category_id"
    );
    let offset = (page - 1) * page_size;

    let (posts, total) = if let Some(tag_id) = tag_id {
        let filter = tenant_filter_aliased_ph("p", tenant_id, 4);
        let sql = format!(
            "SELECT p.* FROM posts p INNER JOIN taggings tg ON p.id = tg.taggable_id WHERE p.status = {} AND tg.taggable_type = {} AND tg.tag_id = {}{filter} ORDER BY p.is_pinned DESC, p.created_at DESC LIMIT {} OFFSET {}",
            Driver::ph(1),
            Driver::ph(2),
            Driver::ph(3),
            Driver::ph(5),
            Driver::ph(6)
        );
        let mut query = sqlx::query_as::<_, Post>(&sql)
            .bind(PostStatus::Published)
            .bind("post")
            .bind(tag_id);
        if let Some(tid) = tenant_id {
            query = query.bind(tid);
        }
        let posts = query.bind(page_size).bind(offset).fetch_all(pool).await?;

        let filter = tenant_filter_aliased_ph("p", tenant_id, 4);
        let sql = format!(
            "SELECT COUNT(*) FROM posts p INNER JOIN taggings tg ON p.id = tg.taggable_id WHERE p.status = {} AND tg.taggable_type = {} AND tg.tag_id = {}{filter}",
            Driver::ph(1),
            Driver::ph(2),
            Driver::ph(3)
        );
        let mut query = sqlx::query_as::<_, (i64,)>(&sql)
            .bind(PostStatus::Published)
            .bind("post")
            .bind(tag_id);
        if let Some(tid) = tenant_id {
            query = query.bind(tid);
        }
        let total = query.fetch_one(pool).await?;

        (posts, total.0)
    } else if let Some(q) = q {
        let pattern = format!("%{q}%");
        let filter = tenant_filter_ph(tenant_id, 2);
        let sql = format!(
            "SELECT * FROM posts WHERE status = {}{filter} AND (title LIKE {} OR content LIKE {}) ORDER BY is_pinned DESC, created_at DESC LIMIT {} OFFSET {}",
            Driver::ph(1),
            Driver::ph(3),
            Driver::ph(4),
            Driver::ph(5),
            Driver::ph(6)
        );
        let mut query = sqlx::query_as::<_, Post>(&sql).bind(PostStatus::Published);
        if let Some(tid) = tenant_id {
            query = query.bind(tid);
        }
        let posts = query
            .bind(&pattern)
            .bind(&pattern)
            .bind(page_size)
            .bind(offset)
            .fetch_all(pool)
            .await?;

        let sql = format!(
            "SELECT COUNT(*) FROM posts WHERE status = {}{filter} AND (title LIKE {} OR content LIKE {})",
            Driver::ph(1),
            Driver::ph(3),
            Driver::ph(4)
        );
        let mut query = sqlx::query_as::<_, (i64,)>(&sql).bind(PostStatus::Published);
        if let Some(tid) = tenant_id {
            query = query.bind(tid);
        }
        let total = query.bind(&pattern).bind(&pattern).fetch_one(pool).await?;

        (posts, total.0)
    } else if let Some(category_id) = category_id {
        let filter = tenant_filter_ph(tenant_id, 3);
        let sql = format!(
            "SELECT * FROM posts WHERE status = {} AND category_id = {}{filter} ORDER BY is_pinned DESC, created_at DESC LIMIT {} OFFSET {}",
            Driver::ph(1),
            Driver::ph(2),
            Driver::ph(4),
            Driver::ph(5)
        );
        let mut query = sqlx::query_as::<_, Post>(&sql)
            .bind(PostStatus::Published)
            .bind(category_id);
        if let Some(tid) = tenant_id {
            query = query.bind(tid);
        }
        let posts = query.bind(page_size).bind(offset).fetch_all(pool).await?;

        let sql = format!(
            "SELECT COUNT(*) FROM posts WHERE status = {} AND category_id = {}{filter}",
            Driver::ph(1),
            Driver::ph(2)
        );
        let mut query = sqlx::query_as::<_, (i64,)>(&sql)
            .bind(PostStatus::Published)
            .bind(category_id);
        if let Some(tid) = tenant_id {
            query = query.bind(tid);
        }
        let total = query.fetch_one(pool).await?;

        (posts, total.0)
    } else {
        let filter = tenant_filter_ph(tenant_id, 2);
        let sql = format!(
            "SELECT * FROM posts WHERE status = {}{filter} ORDER BY is_pinned DESC, created_at DESC LIMIT {} OFFSET {}",
            Driver::ph(1),
            Driver::ph(3),
            Driver::ph(4)
        );
        let mut query = sqlx::query_as::<_, Post>(&sql).bind(PostStatus::Published);
        if let Some(tid) = tenant_id {
            query = query.bind(tid);
        }
        let posts = query.bind(page_size).bind(offset).fetch_all(pool).await?;

        let sql = format!(
            "SELECT COUNT(*) FROM posts WHERE status = {}{filter}",
            Driver::ph(1)
        );
        let mut query = sqlx::query_as::<_, (i64,)>(&sql).bind(PostStatus::Published);
        if let Some(tid) = tenant_id {
            query = query.bind(tid);
        }
        let total = query.fetch_one(pool).await?;

        (posts, total.0)
    };

    Ok((posts, total))
}

pub async fn find_all_joined(
    pool: &crate::db::Pool,
    page: i64,
    page_size: i64,
    status: Option<PostStatus>,
    keyword: Option<&str>,
    tenant_id: Option<&str>,
) -> AppResult<(Vec<PostJoinedRow>, i64)> {
    let kw_pattern = keyword.filter(|k| !k.is_empty()).map(|k| format!("%{k}%"));

    if kw_pattern.is_some() {
        let offset = (page - 1).saturating_mul(page_size);
        let mut conditions = vec!["1=1".to_string()];
        let mut ph_idx = 1usize;

        if status.is_some() {
            let ph = crate::db::Driver::ph(ph_idx);
            ph_idx += 1;
            conditions.push(format!("p.status = {ph}"));
        }

        if kw_pattern.is_some() {
            let ph1 = crate::db::Driver::ph(ph_idx);
            let ph2 = crate::db::Driver::ph(ph_idx + 1);
            ph_idx += 2;
            conditions.push(format!("(p.title LIKE {ph1} OR p.content LIKE {ph2})"));
        }

        let tf = crate::db::tenant::tenant_filter_ph(tenant_id, ph_idx);
        if tenant_id.is_some() {
            ph_idx += 1;
        }

        let where_clause = conditions.join(" AND ");
        let limit_ph = crate::db::Driver::ph(ph_idx);
        let offset_ph = crate::db::Driver::ph(ph_idx + 1);

        let select_cols = "p.id, p.tenant_id, p.title, p.slug, p.content, p.excerpt, p.cover_image, p.image_ids, p.status, p.created_by, p.updated_by, p.category_id, p.view_count, p.is_pinned, p.password, p.comment_status, p.format, p.template, p.meta_title, p.meta_description, p.og_title, p.og_description, p.og_image, p.canonical_url, p.reading_time, p.created_at, p.updated_at, p.published_at, u.username AS author_name, c.name AS category_name";

        let sql = format!(
            "SELECT {select_cols} FROM posts p LEFT JOIN users u ON p.created_by = u.id LEFT JOIN categories c ON p.category_id = c.id WHERE {where_clause}{tf} ORDER BY p.is_pinned DESC, p.created_at DESC LIMIT {limit_ph} OFFSET {offset_ph}"
        );

        let mut q = sqlx::query_as::<_, PostJoinedRow>(&sql);
        if let Some(s) = status {
            q = q.bind(s);
        }
        if let Some(p) = &kw_pattern {
            q = q.bind(p).bind(p);
        }
        if let Some(tid) = tenant_id {
            q = q.bind(tid);
        }
        let items = q.bind(page_size).bind(offset).fetch_all(pool).await?;

        let count_sql = format!("SELECT COUNT(*) FROM posts p WHERE {where_clause}{tf}");
        let mut cq = sqlx::query_scalar::<_, i64>(&count_sql);
        if let Some(s) = status {
            cq = cq.bind(s);
        }
        if let Some(p) = &kw_pattern {
            cq = cq.bind(p).bind(p);
        }
        if let Some(tid) = tenant_id {
            cq = cq.bind(tid);
        }
        let total = cq.fetch_one(pool).await?;

        return Ok((items, total));
    }

    if let Some(s) = status {
        let result = raisfast_derive::crud_join_paged!(
            pool, PostJoinedRow,
            select: [
                "p.id", "p.tenant_id", "p.title", "p.slug",
                "p.content", "p.excerpt", "p.cover_image", "p.image_ids", "p.status",
                "p.created_by", "p.updated_by", "p.category_id", "p.view_count", "p.is_pinned",
                "p.password", "p.comment_status", "p.format", "p.template",
                "p.meta_title", "p.meta_description", "p.og_title", "p.og_description",
                "p.og_image", "p.canonical_url", "p.reading_time",
                "p.created_at", "p.updated_at", "p.published_at",
                "u.username AS author_name", "c.name AS category_name"
            ],
            from: "posts p",
            joins: [
                LEFT "users u" ON "p.created_by = u.id",
                LEFT "categories c" ON "p.category_id = c.id"
            ],
            where: ("p.status", s),
            tenant_alias: "p",
            tenant: tenant_id,
            order_by: "p.is_pinned DESC, p.created_at DESC",
            page: page,
            page_size: page_size
        );
        Ok(result)
    } else {
        let result = raisfast_derive::crud_join_paged!(
            pool, PostJoinedRow,
            select: [
                "p.id", "p.tenant_id", "p.title", "p.slug",
                "p.content", "p.excerpt", "p.cover_image", "p.image_ids", "p.status",
                "p.created_by", "p.updated_by", "p.category_id", "p.view_count", "p.is_pinned",
                "p.password", "p.comment_status", "p.format", "p.template",
                "p.meta_title", "p.meta_description", "p.og_title", "p.og_description",
                "p.og_image", "p.canonical_url", "p.reading_time",
                "p.created_at", "p.updated_at", "p.published_at",
                "u.username AS author_name", "c.name AS category_name"
            ],
            from: "posts p",
            joins: [
                LEFT "users u" ON "p.created_by = u.id",
                LEFT "categories c" ON "p.category_id = c.id"
            ],
            tenant_alias: "p",
            tenant: tenant_id,
            order_by: "p.is_pinned DESC, p.created_at DESC",
            page: page,
            page_size: page_size
        );
        let (data, _) = result;
        let count_filter = match tenant_id {
            Some(_) => " AND tenant_id = ?",
            None => "",
        };
        let count_sql = format!(
            "SELECT COUNT(*) FROM posts WHERE status = ?{}",
            count_filter
        );
        let mut cq = sqlx::query_scalar::<_, i64>(&count_sql).bind(PostStatus::Published);
        if let Some(tid) = tenant_id {
            cq = cq.bind(tid);
        }
        let total = cq.fetch_one(pool).await?;
        Ok((data, total))
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct PostJoinedRow {
    pub id: SnowflakeId,
    pub tenant_id: Option<String>,
    pub title: String,
    pub slug: String,
    pub content: String,
    pub excerpt: Option<String>,
    pub cover_image: Option<String>,
    pub image_ids: Option<String>,
    pub status: PostStatus,
    pub created_by: SnowflakeId,
    pub updated_by: Option<SnowflakeId>,
    pub category_id: Option<SnowflakeId>,
    pub view_count: i64,
    pub is_pinned: bool,
    pub password: Option<String>,
    pub comment_status: CommentOpenStatus,
    pub format: String,
    pub template: String,
    pub meta_title: Option<String>,
    pub meta_description: Option<String>,
    pub og_title: Option<String>,
    pub og_description: Option<String>,
    pub og_image: Option<String>,
    pub canonical_url: Option<String>,
    pub reading_time: i64,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub published_at: Option<Timestamp>,
    pub author_name: Option<String>,
    pub category_name: Option<String>,
}

const JOIN_SQL: &str = "\
    SELECT p.id, p.tenant_id, p.title, p.slug, p.content, p.excerpt, p.cover_image, p.image_ids, p.status, \
    p.created_by, p.updated_by, p.category_id, p.view_count, p.is_pinned, \
    p.password, p.comment_status, p.format, p.template, \
    p.meta_title, p.meta_description, p.og_title, p.og_description, p.og_image, p.canonical_url, p.reading_time, \
    p.created_at, p.updated_at, \
    p.published_at, u.username AS author_name, c.name AS category_name \
    FROM posts p \
    LEFT JOIN users u ON p.created_by = u.id \
    LEFT JOIN categories c ON p.category_id = c.id";

pub async fn find_joined_by_id(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    tenant_id: Option<&str>,
) -> AppResult<PostJoinedRow> {
    raisfast_derive::crud_join!(
        pool, PostJoinedRow,
        select: ["p.id", "p.tenant_id", "p.title", "p.slug", "p.content", "p.excerpt", "p.cover_image", "p.image_ids", "p.status", "p.created_by", "p.updated_by", "p.category_id", "p.view_count", "p.is_pinned", "p.password", "p.comment_status", "p.format", "p.template", "p.meta_title", "p.meta_description", "p.og_title", "p.og_description", "p.og_image", "p.canonical_url", "p.reading_time", "p.created_at", "p.updated_at", "p.published_at", "u.username AS author_name", "c.name AS category_name"],
        from: "posts p",
        joins: [
            LEFT "users u" ON "p.created_by = u.id",
            LEFT "categories c" ON "p.category_id = c.id"
        ],
        where: ("p.id", id),
        tenant_alias: "p",
        tenant: tenant_id,
        method: fetch_one
    ).map_err(Into::into)
}

pub async fn find_published_joined_by_slug(
    pool: &crate::db::Pool,
    slug: &str,
    tenant_id: Option<&str>,
) -> AppResult<PostJoinedRow> {
    raisfast_derive::crud_join!(
        pool, PostJoinedRow,
        select: ["p.id", "p.tenant_id", "p.title", "p.slug", "p.content", "p.excerpt", "p.cover_image", "p.image_ids", "p.status", "p.created_by", "p.updated_by", "p.category_id", "p.view_count", "p.is_pinned", "p.password", "p.comment_status", "p.format", "p.template", "p.meta_title", "p.meta_description", "p.og_title", "p.og_description", "p.og_image", "p.canonical_url", "p.reading_time", "p.created_at", "p.updated_at", "p.published_at", "u.username AS author_name", "c.name AS category_name"],
        from: "posts p",
        joins: [
            LEFT "users u" ON "p.created_by = u.id",
            LEFT "categories c" ON "p.category_id = c.id"
        ],
        where: AND(("p.slug", slug), ("p.status", PostStatus::Published)),
        tenant_alias: "p",
        tenant: tenant_id,
        method: fetch_one
    ).map_err(Into::into)
}

pub async fn get_tags_for_posts(
    pool: &crate::db::Pool,
    post_ids: &[SnowflakeId],
    _tenant_id: Option<&str>,
) -> AppResult<std::collections::HashMap<SnowflakeId, Vec<TagBrief>>> {
    crate::models::tagging::get_tags_for_posts(pool, "post", post_ids).await
}

pub async fn find_joined_by_ids(
    pool: &crate::db::Pool,
    ids: &[i64],
    tenant_id: Option<&str>,
) -> AppResult<Vec<PostJoinedRow>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }

    raisfast_derive::crud_join!(
        pool,
        PostJoinedRow,
        select: [
            "p.id", "p.tenant_id", "p.title", "p.slug",
            "p.content", "p.excerpt", "p.cover_image", "p.image_ids", "p.status",
            "p.created_by", "p.updated_by", "p.category_id", "p.view_count", "p.is_pinned",
            "p.password", "p.comment_status", "p.format", "p.template",
            "p.meta_title", "p.meta_description", "p.og_title", "p.og_description",
            "p.og_image", "p.canonical_url", "p.reading_time",
            "p.created_at", "p.updated_at", "p.published_at",
            "u.username AS author_name", "c.name AS category_name"
        ],
        from: "posts p",
        joins: [
            LEFT "users u" ON "p.created_by = u.id",
            LEFT "categories c" ON "p.category_id = c.id"
        ],
        where: AND(("p.status", PostStatus::Published), ("p.id", IN, ids)),
        tenant_alias: "p",
        tenant: tenant_id,
        order_by: "p.is_pinned DESC, p.created_at DESC",
        method: fetch_all
    )
    .map_err(Into::into)
}

pub async fn count_published_by_ids(
    pool: &crate::db::Pool,
    ids: &[i64],
    tenant_id: Option<&str>,
) -> AppResult<i64> {
    if ids.is_empty() {
        return Ok(0);
    }

    raisfast_derive::crud_count!(
        pool,
        "posts",
        where: AND(("status", PostStatus::Published), ("id", IN, ids)),
        tenant: tenant_id
    )
    .map_err(Into::into)
}

pub async fn find_published_joined(
    pool: &crate::db::Pool,
    page: i64,
    page_size: i64,
    category_id: Option<i64>,
    tag_id: Option<i64>,
    q: Option<&str>,
    tenant_id: Option<&str>,
) -> AppResult<(Vec<PostJoinedRow>, i64)> {
    raisfast_derive::check_schema!(
        "posts",
        "id",
        "title",
        "slug",
        "content",
        "excerpt",
        "cover_image",
        "status",
        "created_by",
        "updated_by",
        "category_id",
        "view_count",
        "is_pinned",
        "password",
        "comment_status",
        "format",
        "template",
        "meta_title",
        "meta_description",
        "og_title",
        "og_description",
        "og_image",
        "canonical_url",
        "reading_time",
        "created_at",
        "updated_at",
        "published_at"
    );
    raisfast_derive::check_schema!("users", "id", "username");
    raisfast_derive::check_schema!("categories", "id", "name");
    let offset = (page - 1) * page_size;

    let (posts, total) = if let Some(tag_id) = tag_id {
        let filter = tenant_filter_aliased_ph("p", tenant_id, 4);
        let sql = format!(
            "{JOIN_SQL} \
             INNER JOIN taggings tg ON p.id = tg.taggable_id \
             WHERE p.status = {} AND tg.taggable_type = {} AND tg.tag_id = {}{filter} \
             ORDER BY p.is_pinned DESC, p.created_at DESC LIMIT {} OFFSET {}",
            Driver::ph(1),
            Driver::ph(2),
            Driver::ph(3),
            Driver::ph(5),
            Driver::ph(6)
        );
        let mut query = sqlx::query_as::<_, PostJoinedRow>(&sql)
            .bind(PostStatus::Published)
            .bind("post")
            .bind(tag_id);
        if let Some(tid) = tenant_id {
            query = query.bind(tid);
        }
        let posts = query.bind(page_size).bind(offset).fetch_all(pool).await?;

        let sql = format!(
            "SELECT COUNT(*) FROM posts p INNER JOIN taggings tg ON p.id = tg.taggable_id WHERE p.status = {} AND tg.taggable_type = {} AND tg.tag_id = {}{filter}",
            Driver::ph(1),
            Driver::ph(2),
            Driver::ph(3)
        );
        let mut query = sqlx::query_as::<_, (i64,)>(&sql)
            .bind(PostStatus::Published)
            .bind("post")
            .bind(tag_id);
        if let Some(tid) = tenant_id {
            query = query.bind(tid);
        }
        let total = query.fetch_one(pool).await?;

        (posts, total.0)
    } else if let Some(q) = q {
        let pattern = format!("%{q}%");
        let filter = tenant_filter_aliased_ph("p", tenant_id, 2);
        let sql = format!(
            "{JOIN_SQL} \
             WHERE p.status = {}{filter} AND (p.title LIKE {} OR p.content LIKE {}) \
             ORDER BY p.is_pinned DESC, p.created_at DESC LIMIT {} OFFSET {}",
            Driver::ph(1),
            Driver::ph(3),
            Driver::ph(4),
            Driver::ph(5),
            Driver::ph(6)
        );
        let mut query = sqlx::query_as::<_, PostJoinedRow>(&sql).bind(PostStatus::Published);
        if let Some(tid) = tenant_id {
            query = query.bind(tid);
        }
        let posts = query
            .bind(&pattern)
            .bind(&pattern)
            .bind(page_size)
            .bind(offset)
            .fetch_all(pool)
            .await?;

        let filter = tenant_filter_ph(tenant_id, 2);
        let sql = format!(
            "SELECT COUNT(*) FROM posts WHERE status = {}{filter} AND (title LIKE {} OR content LIKE {})",
            Driver::ph(1),
            Driver::ph(3),
            Driver::ph(4)
        );
        let mut query = sqlx::query_as::<_, (i64,)>(&sql).bind(PostStatus::Published);
        if let Some(tid) = tenant_id {
            query = query.bind(tid);
        }
        let total = query.bind(&pattern).bind(&pattern).fetch_one(pool).await?;

        (posts, total.0)
    } else if let Some(category_id) = category_id {
        let filter = tenant_filter_aliased_ph("p", tenant_id, 3);
        let sql = format!(
            "{JOIN_SQL} \
             WHERE p.status = {} AND p.category_id = {}{filter} \
             ORDER BY p.is_pinned DESC, p.created_at DESC LIMIT {} OFFSET {}",
            Driver::ph(1),
            Driver::ph(2),
            Driver::ph(4),
            Driver::ph(5)
        );
        let mut query = sqlx::query_as::<_, PostJoinedRow>(&sql)
            .bind(PostStatus::Published)
            .bind(category_id);
        if let Some(tid) = tenant_id {
            query = query.bind(tid);
        }
        let posts = query.bind(page_size).bind(offset).fetch_all(pool).await?;

        let filter = tenant_filter_ph(tenant_id, 3);
        let sql = format!(
            "SELECT COUNT(*) FROM posts WHERE status = {} AND category_id = {}{filter}",
            Driver::ph(1),
            Driver::ph(2)
        );
        let mut query = sqlx::query_as::<_, (i64,)>(&sql)
            .bind(PostStatus::Published)
            .bind(category_id);
        if let Some(tid) = tenant_id {
            query = query.bind(tid);
        }
        let total = query.fetch_one(pool).await?;

        (posts, total.0)
    } else {
        let filter = tenant_filter_aliased_ph("p", tenant_id, 2);
        let sql = format!(
            "{JOIN_SQL} \
             WHERE p.status = {}{filter} \
             ORDER BY p.is_pinned DESC, p.created_at DESC LIMIT {} OFFSET {}",
            Driver::ph(1),
            Driver::ph(3),
            Driver::ph(4)
        );
        let mut query = sqlx::query_as::<_, PostJoinedRow>(&sql).bind(PostStatus::Published);
        if let Some(tid) = tenant_id {
            query = query.bind(tid);
        }
        let posts = query.bind(page_size).bind(offset).fetch_all(pool).await?;

        let filter = tenant_filter_ph(tenant_id, 2);
        let sql = format!(
            "SELECT COUNT(*) FROM posts WHERE status = {}{filter}",
            Driver::ph(1)
        );
        let mut query = sqlx::query_as::<_, (i64,)>(&sql).bind(PostStatus::Published);
        if let Some(tid) = tenant_id {
            query = query.bind(tid);
        }
        let total = query.fetch_one(pool).await?;

        (posts, total.0)
    };

    Ok((posts, total))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::CreatePostCmd;

    async fn setup_pool() -> crate::db::Pool {
        crate::test_pool!()
    }

    async fn create_user(pool: &crate::db::Pool) -> i64 {
        let uid = crate::utils::id::new_id();
        sqlx::query(
            "INSERT INTO users (id, username, role, status, registered_via) VALUES (?, 'testuser', 'author', 'active', 'email')",
        )
        .bind(uid)
        .execute(pool)
        .await
        .unwrap();
        uid
    }

    async fn create_test_post(
        pool: &crate::db::Pool,
        created_by: i64,
        status: &str,
        title: &str,
    ) -> Post {
        create(
            pool,
            &CreatePostCmd {
                title: title.to_string(),
                slug: title.to_lowercase().replace(' ', "-"),
                content: format!("Content of {title}"),
                excerpt: None,
                cover_image: None,
                image_ids: None,
                status: status.parse().unwrap(),
                created_by,
                updated_by: Some(created_by),
                category_id: None,
                tag_ids: None,
                meta_title: None,
                meta_description: None,
                og_title: None,
                og_description: None,
                og_image: None,
                canonical_url: None,
            },
            None,
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn find_joined_by_ids_empty() {
        let pool = setup_pool().await;
        let result = find_joined_by_ids(&pool, &[], None).await.unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn find_joined_by_ids_single() {
        let pool = setup_pool().await;
        let uid = create_user(&pool).await;
        let p = create_test_post(&pool, uid, "published", "Test Post").await;
        let result = find_joined_by_ids(&pool, &[*p.id], None).await.unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, p.id);
        assert_eq!(result[0].title, "Test Post");
        assert_eq!(result[0].author_name.as_deref(), Some("testuser"));
    }

    #[tokio::test]
    async fn find_joined_by_ids_multiple() {
        let pool = setup_pool().await;
        let uid = create_user(&pool).await;
        let p1 = create_test_post(&pool, uid, "published", "Post A").await;
        let p2 = create_test_post(&pool, uid, "published", "Post B").await;
        let p3 = create_test_post(&pool, uid, "published", "Post C").await;
        let result = find_joined_by_ids(&pool, &[*p1.id, *p3.id], None)
            .await
            .unwrap();
        assert_eq!(result.len(), 2);
        let ids: Vec<i64> = result.iter().map(|r| *r.id).collect();
        assert!(ids.contains(&p1.id));
        assert!(ids.contains(&p3.id));
        assert!(!ids.contains(&p2.id));
    }

    #[tokio::test]
    async fn find_joined_by_ids_filters_draft() {
        let pool = setup_pool().await;
        let uid = create_user(&pool).await;
        let p = create_test_post(&pool, uid, "draft", "Draft Post").await;
        let result = find_joined_by_ids(&pool, &[*p.id], None).await.unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn find_joined_by_ids_nonexistent() {
        let pool = setup_pool().await;
        let result = find_joined_by_ids(&pool, &[-1], None).await.unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn find_joined_by_ids_mixed_published_and_draft() {
        let pool = setup_pool().await;
        let uid = create_user(&pool).await;
        let pub_post = create_test_post(&pool, uid, "published", "Published").await;
        let draft_post = create_test_post(&pool, uid, "draft", "Draft").await;
        let result = find_joined_by_ids(&pool, &[*pub_post.id, *draft_post.id], None)
            .await
            .unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].title, "Published");
    }

    #[tokio::test]
    async fn find_joined_by_ids_with_category() {
        let pool = setup_pool().await;
        let uid = create_user(&pool).await;
        let cat_id = crate::utils::id::new_id();
        sqlx::query("INSERT INTO categories (id, name, slug) VALUES (?, 'Tech', 'tech')")
            .bind(cat_id)
            .execute(&pool)
            .await
            .unwrap();
        let p = create(
            &pool,
            &CreatePostCmd {
                title: "Category Post".to_string(),
                slug: "cat-post".to_string(),
                content: "Content".to_string(),
                excerpt: None,
                cover_image: None,
                image_ids: None,
                status: PostStatus::Published,
                created_by: uid,
                updated_by: Some(uid),
                category_id: Some(cat_id),
                tag_ids: None,
                meta_title: None,
                meta_description: None,
                og_title: None,
                og_description: None,
                og_image: None,
                canonical_url: None,
            },
            None,
        )
        .await
        .unwrap();
        let result = find_joined_by_ids(&pool, &[*p.id], None).await.unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].category_name.as_deref(), Some("Tech"));
    }

    #[tokio::test]
    async fn count_published_by_ids_empty() {
        let pool = setup_pool().await;
        let count = count_published_by_ids(&pool, &[], None).await.unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn count_published_by_ids_single() {
        let pool = setup_pool().await;
        let uid = create_user(&pool).await;
        let p = create_test_post(&pool, uid, "published", "Count Post").await;
        let count = count_published_by_ids(&pool, &[*p.id], None).await.unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn count_published_by_ids_filters_draft() {
        let pool = setup_pool().await;
        let uid = create_user(&pool).await;
        let p = create_test_post(&pool, uid, "draft", "Draft").await;
        let count = count_published_by_ids(&pool, &[*p.id], None).await.unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn count_published_by_ids_multiple() {
        let pool = setup_pool().await;
        let uid = create_user(&pool).await;
        let p1 = create_test_post(&pool, uid, "published", "A").await;
        let p2 = create_test_post(&pool, uid, "draft", "B").await;
        let p3 = create_test_post(&pool, uid, "published", "C").await;
        let count = count_published_by_ids(&pool, &[*p1.id, *p2.id, *p3.id], None)
            .await
            .unwrap();
        assert_eq!(count, 2);
    }

    #[tokio::test]
    async fn count_published_by_ids_nonexistent() {
        let pool = setup_pool().await;
        let count = count_published_by_ids(&pool, &[-1], None).await.unwrap();
        assert_eq!(count, 0);
    }
}
