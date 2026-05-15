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

use crate::db::dialect::ph;
use crate::db::tenant::{tenant_filter_aliased_ph, tenant_filter_ph};
use crate::errors::app_error::{AppError, AppResult};
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

#[derive(Debug, Serialize, Deserialize, Clone)]
#[non_exhaustive]
pub struct Post {
    pub id: i64,
    pub document_id: String,
    pub tenant_id: Option<String>,
    pub title: String,
    pub slug: String,
    pub content: String,
    pub excerpt: Option<String>,
    pub cover_image: Option<String>,
    pub status: PostStatus,
    pub created_by: i64,
    pub updated_by: Option<i64>,
    pub category_id: Option<i64>,
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

crate::impl_from_row_opt_tenant!(Post {
    required { id, document_id, title, slug, content, status, created_by, view_count, is_pinned, comment_status, format, template, reading_time, created_at, updated_at }
    optional { excerpt, cover_image, updated_by, category_id, published_at, password, meta_title, meta_description, og_title, og_description, og_image, canonical_url }
});

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, Deserialize, Clone, utoipa::ToSchema)]
pub struct TagBrief {
    pub id: i64,
    pub name: String,
    pub slug: String,
}

pub async fn find_by_slug(
    pool: &crate::db::Pool,
    slug: &str,
    tenant_id: Option<&str>,
) -> AppResult<Option<Post>> {
    let sql = format!(
        "SELECT * FROM posts WHERE slug = {}{}",
        ph(1),
        tenant_filter_ph(tenant_id, 2)
    );
    let mut q = sqlx::query_as::<_, Post>(&sql).bind(slug);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    let post = q.fetch_optional(pool).await?;
    Ok(post)
}

pub async fn find_by_id(
    pool: &crate::db::Pool,
    id: i64,
    tenant_id: Option<&str>,
) -> AppResult<Option<Post>> {
    let sql = format!(
        "SELECT * FROM posts WHERE id = {}{}",
        ph(1),
        tenant_filter_ph(tenant_id, 2)
    );
    let mut q = sqlx::query_as::<_, Post>(&sql).bind(id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    let post = q.fetch_optional(pool).await?;
    Ok(post)
}

pub async fn find_by_document_id(
    pool: &crate::db::Pool,
    document_id: &str,
    tenant_id: Option<&str>,
) -> AppResult<Option<Post>> {
    let sql = format!(
        "SELECT * FROM posts WHERE document_id = {}{}",
        ph(1),
        tenant_filter_ph(tenant_id, 2)
    );
    let mut q = sqlx::query_as::<_, Post>(&sql).bind(document_id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    let post = q.fetch_optional(pool).await?;
    Ok(post)
}

pub async fn create(
    pool: &crate::db::Pool,
    cmd: &crate::commands::CreatePostCmd,
    tenant_id: Option<&str>,
) -> AppResult<Post> {
    let mut tx = pool.begin().await?;
    let post = create_tx(&mut tx, cmd, tenant_id).await?;
    let doc_id = post.document_id.clone();
    tx.commit().await?;
    find_by_document_id(pool, &doc_id, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found("post"))
}

pub async fn create_tx(
    tx: &mut crate::db::Transaction<'_>,
    cmd: &crate::commands::CreatePostCmd,
    tenant_id: Option<&str>,
) -> AppResult<Post> {
    let (document_id, now) = crate::utils::id::new_document_id_and_timestamp();
    let published_at = if cmd.status == PostStatus::Published {
        Some(now)
    } else {
        None
    };
    match tenant_id {
        Some(tid) => {
            let sql = format!(
                "INSERT INTO posts (document_id, tenant_id, title, slug, content, excerpt, cover_image, status, created_by, updated_by, category_id, published_at, created_at, updated_at) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
                ph(1),
                ph(2),
                ph(3),
                ph(4),
                ph(5),
                ph(6),
                ph(7),
                ph(8),
                ph(9),
                ph(10),
                ph(11),
                ph(12),
                ph(13),
                ph(14)
            );
            sqlx::query(&sql)
                .bind(&document_id)
                .bind(tid)
                .bind(&cmd.title)
                .bind(&cmd.slug)
                .bind(&cmd.content)
                .bind(&cmd.excerpt)
                .bind(&cmd.cover_image)
                .bind(cmd.status)
                .bind(cmd.created_by)
                .bind(cmd.updated_by)
                .bind(cmd.category_id)
                .bind(published_at)
                .bind(now)
                .bind(now)
                .execute(&mut **tx)
                .await?;
        }
        None => {
            let sql = format!(
                "INSERT INTO posts (document_id, title, slug, content, excerpt, cover_image, status, created_by, updated_by, category_id, published_at, created_at, updated_at) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
                ph(1),
                ph(2),
                ph(3),
                ph(4),
                ph(5),
                ph(6),
                ph(7),
                ph(8),
                ph(9),
                ph(10),
                ph(11),
                ph(12),
                ph(13)
            );
            sqlx::query(&sql)
                .bind(&document_id)
                .bind(&cmd.title)
                .bind(&cmd.slug)
                .bind(&cmd.content)
                .bind(&cmd.excerpt)
                .bind(&cmd.cover_image)
                .bind(cmd.status)
                .bind(cmd.created_by)
                .bind(cmd.updated_by)
                .bind(cmd.category_id)
                .bind(published_at)
                .bind(now)
                .bind(now)
                .execute(&mut **tx)
                .await?;
        }
    }

    let created = find_by_document_id_tx(tx, &document_id, tenant_id)
        .await?
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("failed to read created post")))?;

    Ok(created)
}

async fn find_by_document_id_tx(
    tx: &mut crate::db::Transaction<'_>,
    document_id: &str,
    tenant_id: Option<&str>,
) -> AppResult<Option<Post>> {
    let filter = tenant_filter_ph(tenant_id, 2);
    let sql = format!("SELECT * FROM posts WHERE document_id = {}{filter}", ph(1));
    let mut q = sqlx::query_as::<_, Post>(&sql).bind(document_id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    q.fetch_optional(&mut **tx).await.map_err(Into::into)
}

pub async fn update(
    pool: &crate::db::Pool,
    cmd: &crate::commands::UpdatePostCmd,
    tenant_id: Option<&str>,
) -> AppResult<Post> {
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
    let post_id: i64 = cmd.id;
    let sql = format!(
        "SELECT * FROM posts WHERE id = {}{}",
        ph(1),
        tenant_filter_ph(tenant_id, 2)
    );
    let mut q = sqlx::query_as::<_, Post>(&sql).bind(post_id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    let existing = q
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(|| AppError::not_found("post"))?;

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
    let category_id: Option<i64> = cmd.category_id.or(existing.category_id);
    let slug = cmd.slug.as_deref().unwrap_or(&existing.slug);
    let updated_by: Option<i64> = cmd.updated_by.or(existing.updated_by);

    let sql = format!(
        "UPDATE posts SET title = {}, slug = {}, content = {}, excerpt = {}, cover_image = {}, status = {}, category_id = {}, published_at = {}, updated_by = {}, updated_at = {} WHERE id = {}{}",
        ph(1),
        ph(2),
        ph(3),
        ph(4),
        ph(5),
        ph(6),
        ph(7),
        ph(8),
        ph(9),
        ph(10),
        ph(11),
        tenant_filter_ph(tenant_id, 12)
    );
    let mut q = sqlx::query(&sql)
        .bind(title)
        .bind(slug)
        .bind(content)
        .bind(&excerpt)
        .bind(&cover_image)
        .bind(new_status)
        .bind(category_id)
        .bind(published_at)
        .bind(updated_by)
        .bind(now)
        .bind(post_id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    q.execute(&mut **tx).await?;

    Ok(Post {
        id: existing.id,
        document_id: existing.document_id,
        tenant_id: existing.tenant_id,
        title: title.to_string(),
        slug: slug.to_string(),
        content: content.to_string(),
        excerpt,
        cover_image,
        status: new_status,
        created_by: existing.created_by,
        updated_by,
        category_id,
        view_count: existing.view_count,
        is_pinned: existing.is_pinned,
        password: existing.password,
        comment_status: existing.comment_status,
        format: existing.format,
        template: existing.template,
        meta_title: existing.meta_title,
        meta_description: existing.meta_description,
        og_title: existing.og_title,
        og_description: existing.og_description,
        og_image: existing.og_image,
        canonical_url: existing.canonical_url,
        reading_time: existing.reading_time,
        created_at: existing.created_at,
        updated_at: now,
        published_at,
    })
}

pub async fn delete(pool: &crate::db::Pool, id: i64, tenant_id: Option<&str>) -> AppResult<()> {
    let sql = format!(
        "DELETE FROM posts WHERE id = {}{}",
        ph(1),
        tenant_filter_ph(tenant_id, 2)
    );
    let mut q = sqlx::query(&sql).bind(id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    let result = q.execute(pool).await?;

    AppError::expect_affected(&result, "post")
}

pub async fn increment_view_count_joined(
    pool: &crate::db::Pool,
    slug: &str,
    tenant_id: Option<&str>,
) -> AppResult<PostJoinedRow> {
    let sql = format!(
        "UPDATE posts SET view_count = view_count + 1 WHERE slug = {} AND status = {}{}",
        ph(1),
        ph(2),
        tenant_filter_ph(tenant_id, 3)
    );
    let mut q = sqlx::query(&sql).bind(slug).bind(PostStatus::Published);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    q.execute(pool).await?;

    find_published_joined_by_slug(pool, slug, tenant_id).await
}

pub async fn sync_tags(pool: &crate::db::Pool, post_id: i64, tag_ids: &[i64]) -> AppResult<()> {
    let mut tx = pool.begin().await?;
    sync_tags_tx(&mut tx, post_id, tag_ids).await?;
    tx.commit().await?;
    Ok(())
}

pub async fn sync_tags_tx(
    tx: &mut crate::db::Transaction<'_>,
    post_id: i64,
    tag_ids: &[i64],
) -> AppResult<()> {
    let sql = format!("DELETE FROM posts_tags WHERE post_id = {}", ph(1));
    sqlx::query(&sql).bind(post_id).execute(&mut **tx).await?;

    for tag_id in tag_ids {
        let sql = format!(
            "INSERT INTO posts_tags (post_id, tag_id) VALUES ({}, {})",
            ph(1),
            ph(2)
        );
        sqlx::query(&sql)
            .bind(post_id)
            .bind(tag_id)
            .execute(&mut **tx)
            .await?;
    }

    Ok(())
}

#[derive(Debug, FromRow)]
pub struct TagRow {
    pub id: i64,
    pub name: String,
    pub slug: String,
}

pub async fn get_post_tags(
    pool: &crate::db::Pool,
    post_id: i64,
    tenant_id: Option<&str>,
) -> AppResult<Vec<TagBrief>> {
    let sql = format!(
        "SELECT t.id, t.name, t.slug FROM tags t INNER JOIN posts_tags pt ON t.id = pt.tag_id WHERE pt.post_id = {}{}",
        ph(1),
        tenant_filter_aliased_ph("t", tenant_id, 2)
    );
    let mut q = sqlx::query_as::<_, TagRow>(&sql).bind(post_id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    let rows = q.fetch_all(pool).await?;

    Ok(rows
        .into_iter()
        .map(|r| TagBrief {
            id: r.id,
            name: r.name,
            slug: r.slug,
        })
        .collect())
}

#[derive(Debug, FromRow)]
pub struct AuthorRow {
    pub username: String,
}

pub async fn get_author_name(
    pool: &crate::db::Pool,
    created_by: i64,
    tenant_id: Option<&str>,
) -> AppResult<Option<String>> {
    let sql = format!(
        "SELECT username FROM users WHERE id = {}{}",
        ph(1),
        tenant_filter_ph(tenant_id, 2)
    );
    let mut q = sqlx::query_as::<_, AuthorRow>(&sql).bind(created_by);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    let row = q.fetch_optional(pool).await?;
    Ok(row.map(|r| r.username))
}

#[derive(Debug, FromRow)]
pub struct CategoryNameRow {
    pub name: String,
}

pub async fn get_category_name(
    pool: &crate::db::Pool,
    category_id: i64,
    tenant_id: Option<&str>,
) -> AppResult<Option<String>> {
    let sql = format!(
        "SELECT name FROM categories WHERE id = {}{}",
        ph(1),
        tenant_filter_ph(tenant_id, 2)
    );
    let mut q = sqlx::query_as::<_, CategoryNameRow>(&sql).bind(category_id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    let row = q.fetch_optional(pool).await?;
    Ok(row.map(|r| r.name))
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
    let offset = (page - 1) * page_size;

    let (posts, total) = if let Some(tag_id) = tag_id {
        let filter = tenant_filter_aliased_ph("p", tenant_id, 3);
        let sql = format!(
            "SELECT p.* FROM posts p INNER JOIN posts_tags pt ON p.id = pt.post_id WHERE p.status = {} AND pt.tag_id = {}{filter} ORDER BY p.is_pinned DESC, p.created_at DESC LIMIT {} OFFSET {}",
            ph(1),
            ph(2),
            ph(4),
            ph(5)
        );
        let mut query = sqlx::query_as::<_, Post>(&sql)
            .bind(PostStatus::Published)
            .bind(tag_id);
        if let Some(tid) = tenant_id {
            query = query.bind(tid);
        }
        let posts = query.bind(page_size).bind(offset).fetch_all(pool).await?;

        let filter = tenant_filter_aliased_ph("p", tenant_id, 3);
        let sql = format!(
            "SELECT COUNT(*) FROM posts p INNER JOIN posts_tags pt ON p.id = pt.post_id WHERE p.status = {} AND pt.tag_id = {}{filter}",
            ph(1),
            ph(2)
        );
        let mut query = sqlx::query_as::<_, (i64,)>(&sql)
            .bind(PostStatus::Published)
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
            ph(1),
            ph(3),
            ph(4),
            ph(5),
            ph(6)
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
            ph(1),
            ph(3),
            ph(4)
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
            ph(1),
            ph(2),
            ph(4),
            ph(5)
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
            ph(1),
            ph(2)
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
            ph(1),
            ph(3),
            ph(4)
        );
        let mut query = sqlx::query_as::<_, Post>(&sql).bind(PostStatus::Published);
        if let Some(tid) = tenant_id {
            query = query.bind(tid);
        }
        let posts = query.bind(page_size).bind(offset).fetch_all(pool).await?;

        let sql = format!(
            "SELECT COUNT(*) FROM posts WHERE status = {}{filter}",
            ph(1)
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
    tenant_id: Option<&str>,
) -> AppResult<(Vec<PostJoinedRow>, i64)> {
    let offset = (page - 1) * page_size;

    let (posts, total) = if let Some(status) = status {
        let filter = tenant_filter_aliased_ph("p", tenant_id, 2);
        let sql = format!(
            "{JOIN_SQL} \
             WHERE p.status = {}{filter} \
             ORDER BY p.is_pinned DESC, p.created_at DESC LIMIT {} OFFSET {}",
            ph(1),
            ph(3),
            ph(4)
        );
        let mut query = sqlx::query_as::<_, PostJoinedRow>(&sql).bind(status);
        if let Some(tid) = tenant_id {
            query = query.bind(tid);
        }
        let posts = query.bind(page_size).bind(offset).fetch_all(pool).await?;
        let filter = tenant_filter_ph(tenant_id, 2);
        let sql = format!(
            "SELECT COUNT(*) FROM posts WHERE status = {}{filter}",
            ph(1)
        );
        let mut query = sqlx::query_as::<_, (i64,)>(&sql).bind(status);
        if let Some(tid) = tenant_id {
            query = query.bind(tid);
        }
        let total = query.fetch_one(pool).await?;
        (posts, total.0)
    } else {
        let filter = tenant_filter_aliased_ph("p", tenant_id, 1);
        let sql = format!(
            "{JOIN_SQL} \
             WHERE 1=1{filter} \
             ORDER BY p.is_pinned DESC, p.created_at DESC LIMIT {} OFFSET {}",
            ph(2),
            ph(3)
        );
        let mut query = sqlx::query_as::<_, PostJoinedRow>(&sql);
        if let Some(tid) = tenant_id {
            query = query.bind(tid);
        }
        let posts = query.bind(page_size).bind(offset).fetch_all(pool).await?;
        let filter = tenant_filter_ph(tenant_id, 2);
        let sql = format!(
            "SELECT COUNT(*) FROM posts WHERE status = {}{filter}",
            ph(1)
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
        let uid = uuid::Uuid::now_v7().to_string();
        sqlx::query(
            "INSERT INTO users (document_id, username, role, status, registered_via) VALUES (?, 'testuser', 'author', 'active', 'email')",
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
                status: status.parse().unwrap(),
                created_by: created_by,
                updated_by: Some(created_by),
                category_id: None,
                tag_ids: None,
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
        let result = find_joined_by_ids(&pool, &[p.id], None).await.unwrap();
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
        let result = find_joined_by_ids(&pool, &[p1.id, p3.id], None)
            .await
            .unwrap();
        assert_eq!(result.len(), 2);
        let ids: Vec<i64> = result.iter().map(|r| r.id).collect();
        assert!(ids.contains(&p1.id));
        assert!(ids.contains(&p3.id));
        assert!(!ids.contains(&p2.id));
    }

    #[tokio::test]
    async fn find_joined_by_ids_filters_draft() {
        let pool = setup_pool().await;
        let uid = create_user(&pool).await;
        let p = create_test_post(&pool, uid, "draft", "Draft Post").await;
        let result = find_joined_by_ids(&pool, &[p.id], None).await.unwrap();
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
        let result = find_joined_by_ids(&pool, &[pub_post.id, draft_post.id], None)
            .await
            .unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].title, "Published");
    }

    #[tokio::test]
    async fn find_joined_by_ids_with_category() {
        let pool = setup_pool().await;
        let uid = create_user(&pool).await;
        let cat_doc_id = uuid::Uuid::now_v7().to_string();
        sqlx::query("INSERT INTO categories (document_id, name, slug) VALUES (?, 'Tech', 'tech')")
            .bind(&cat_doc_id)
            .execute(&pool)
            .await
            .unwrap();
        let (cat_int_id,): (i64,) =
            sqlx::query_as("SELECT id FROM categories WHERE document_id = ?")
                .bind(&cat_doc_id)
                .fetch_one(&pool)
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
                status: PostStatus::Published,
                created_by: uid,
                updated_by: Some(uid),
                category_id: Some(cat_int_id),
                tag_ids: None,
            },
            None,
        )
        .await
        .unwrap();
        let result = find_joined_by_ids(&pool, &[p.id], None).await.unwrap();
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
        let count = count_published_by_ids(&pool, &[p.id], None).await.unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn count_published_by_ids_filters_draft() {
        let pool = setup_pool().await;
        let uid = create_user(&pool).await;
        let p = create_test_post(&pool, uid, "draft", "Draft").await;
        let count = count_published_by_ids(&pool, &[p.id], None).await.unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn count_published_by_ids_multiple() {
        let pool = setup_pool().await;
        let uid = create_user(&pool).await;
        let p1 = create_test_post(&pool, uid, "published", "A").await;
        let p2 = create_test_post(&pool, uid, "draft", "B").await;
        let p3 = create_test_post(&pool, uid, "published", "C").await;
        let count = count_published_by_ids(&pool, &[p1.id, p2.id, p3.id], None)
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

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PostJoinedRow {
    pub id: i64,
    pub document_id: String,
    pub tenant_id: Option<String>,
    pub title: String,
    pub slug: String,
    pub content: String,
    pub excerpt: Option<String>,
    pub cover_image: Option<String>,
    pub status: PostStatus,
    pub created_by: i64,
    pub updated_by: Option<i64>,
    pub category_id: Option<i64>,
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

crate::impl_from_row_opt_tenant!(PostJoinedRow {
    required { id, document_id, title, slug, content, status, created_by, view_count, is_pinned, comment_status, format, template, reading_time, created_at, updated_at }
    optional { excerpt, cover_image, updated_by, category_id, published_at, author_name, category_name, password, meta_title, meta_description, og_title, og_description, og_image, canonical_url }
});

const JOIN_SQL: &str = "\
    SELECT p.id, p.document_id, p.title, p.slug, p.content, p.excerpt, p.cover_image, p.status, \
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
    id: i64,
    tenant_id: Option<&str>,
) -> AppResult<PostJoinedRow> {
    let sql = format!(
        "{JOIN_SQL} WHERE p.id = {}{}",
        ph(1),
        tenant_filter_aliased_ph("p", tenant_id, 2)
    );
    let mut q = sqlx::query_as::<_, PostJoinedRow>(&sql).bind(id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    q.fetch_one(pool).await.map_err(Into::into)
}

pub async fn find_published_joined_by_slug(
    pool: &crate::db::Pool,
    slug: &str,
    tenant_id: Option<&str>,
) -> AppResult<PostJoinedRow> {
    let sql = format!(
        "{JOIN_SQL} WHERE p.slug = {} AND p.status = {}{}",
        ph(1),
        ph(2),
        tenant_filter_aliased_ph("p", tenant_id, 3)
    );
    let mut q = sqlx::query_as::<_, PostJoinedRow>(&sql)
        .bind(slug)
        .bind(PostStatus::Published);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    q.fetch_one(pool).await.map_err(Into::into)
}

pub async fn get_tags_for_posts(
    pool: &crate::db::Pool,
    post_ids: &[i64],
    tenant_id: Option<&str>,
) -> AppResult<std::collections::HashMap<i64, Vec<TagBrief>>> {
    if post_ids.is_empty() {
        return Ok(std::collections::HashMap::new());
    }

    let placeholders: Vec<String> = (1..=post_ids.len()).map(ph).collect();
    let next_idx = post_ids.len() + 1;
    let sql = format!(
        "SELECT pt.post_id, t.id, t.name, t.slug \
         FROM posts_tags pt \
         JOIN tags t ON pt.tag_id = t.id \
         WHERE pt.post_id IN ({}){}",
        placeholders.join(","),
        tenant_filter_aliased_ph("t", tenant_id, next_idx)
    );

    #[derive(Debug, FromRow)]
    struct TagWithPostId {
        post_id: i64,
        id: i64,
        name: String,
        slug: String,
    }

    let mut query = sqlx::query_as::<_, TagWithPostId>(&sql);
    for id in post_ids {
        query = query.bind(id);
    }
    if let Some(tid) = tenant_id {
        query = query.bind(tid);
    }
    let rows = query.fetch_all(pool).await?;

    let mut map: std::collections::HashMap<i64, Vec<TagBrief>> = std::collections::HashMap::new();
    for row in rows {
        map.entry(row.post_id).or_default().push(TagBrief {
            id: row.id,
            name: row.name,
            slug: row.slug,
        });
    }
    Ok(map)
}

pub async fn find_joined_by_ids(
    pool: &crate::db::Pool,
    ids: &[i64],
    tenant_id: Option<&str>,
) -> AppResult<Vec<PostJoinedRow>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }

    let placeholders: Vec<String> = (1..=ids.len()).map(ph).collect();
    let next_idx = ids.len() + 1;
    let sql = format!(
        "{} \
         WHERE p.id IN ({}) AND p.status = {}{} \
         ORDER BY p.is_pinned DESC, p.created_at DESC",
        JOIN_SQL,
        placeholders.join(","),
        ph(next_idx),
        tenant_filter_aliased_ph("p", tenant_id, next_idx + 1)
    );

    let mut query = sqlx::query_as::<_, PostJoinedRow>(&sql);
    for id in ids {
        query = query.bind(id);
    }
    query = query.bind(PostStatus::Published);
    if let Some(tid) = tenant_id {
        query = query.bind(tid);
    }
    let rows = query.fetch_all(pool).await?;
    Ok(rows)
}

pub async fn count_published_by_ids(
    pool: &crate::db::Pool,
    ids: &[i64],
    tenant_id: Option<&str>,
) -> AppResult<i64> {
    if ids.is_empty() {
        return Ok(0);
    }

    let placeholders: Vec<String> = (1..=ids.len()).map(ph).collect();
    let next_idx = ids.len() + 1;
    let sql = format!(
        "SELECT COUNT(*) FROM posts WHERE id IN ({}) AND status = {}{}",
        placeholders.join(","),
        ph(next_idx),
        tenant_filter_ph(tenant_id, next_idx + 1)
    );
    let mut query = sqlx::query_as::<_, (i64,)>(&sql);
    for id in ids {
        query = query.bind(id);
    }
    query = query.bind(PostStatus::Published);
    if let Some(tid) = tenant_id {
        query = query.bind(tid);
    }
    let (count,) = query.fetch_one(pool).await?;
    Ok(count)
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
    let offset = (page - 1) * page_size;

    let (posts, total) = if let Some(tag_id) = tag_id {
        let filter = tenant_filter_aliased_ph("p", tenant_id, 3);
        let sql = format!(
            "{JOIN_SQL} \
             INNER JOIN posts_tags pt ON p.id = pt.post_id \
             WHERE p.status = {} AND pt.tag_id = {}{filter} \
             ORDER BY p.is_pinned DESC, p.created_at DESC LIMIT {} OFFSET {}",
            ph(1),
            ph(2),
            ph(4),
            ph(5)
        );
        let mut query = sqlx::query_as::<_, PostJoinedRow>(&sql)
            .bind(PostStatus::Published)
            .bind(tag_id);
        if let Some(tid) = tenant_id {
            query = query.bind(tid);
        }
        let posts = query.bind(page_size).bind(offset).fetch_all(pool).await?;

        let sql = format!(
            "SELECT COUNT(*) FROM posts p INNER JOIN posts_tags pt ON p.id = pt.post_id WHERE p.status = {} AND pt.tag_id = {}{filter}",
            ph(1),
            ph(2)
        );
        let mut query = sqlx::query_as::<_, (i64,)>(&sql)
            .bind(PostStatus::Published)
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
            ph(1),
            ph(3),
            ph(4),
            ph(5),
            ph(6)
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
            ph(1),
            ph(3),
            ph(4)
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
            ph(1),
            ph(2),
            ph(4),
            ph(5)
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
            ph(1),
            ph(2)
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
            ph(1),
            ph(3),
            ph(4)
        );
        let mut query = sqlx::query_as::<_, PostJoinedRow>(&sql).bind(PostStatus::Published);
        if let Some(tid) = tenant_id {
            query = query.bind(tid);
        }
        let posts = query.bind(page_size).bind(offset).fetch_all(pool).await?;

        let filter = tenant_filter_ph(tenant_id, 2);
        let sql = format!(
            "SELECT COUNT(*) FROM posts WHERE status = {}{filter}",
            ph(1)
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
