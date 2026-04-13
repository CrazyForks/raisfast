//! 文章模型与数据库查询
//!
//! 定义文章（Post）相关的数据结构，包括完整行模型、面向前端的响应模型、
//! 标签摘要结构体，以及对 `posts` 表和关联表的全部增删改查操作。
//!
//! 同时提供获取作者名、分类名、文章标签等关联数据的辅助查询函数，
//! 以及按分类/标签/关键词筛选已发布文章的分页查询。

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;
use validator::Validate;

use crate::errors::app_error::{AppError, AppResult};

/// 文章完整数据库行模型
///
/// 直接映射 `posts` 表的所有字段。
/// 首次发布时自动填充 `published_at`；`status` 可取 `draft`、`published` 等。
#[derive(Debug, FromRow, Serialize, Deserialize, Clone)]
#[non_exhaustive]
pub struct Post {
    pub id: String,
    pub title: String,
    pub slug: String,
    pub content: String,
    pub excerpt: Option<String>,
    pub cover_image: Option<String>,
    pub status: String,
    pub author_id: String,
    pub category_id: Option<String>,
    pub view_count: i64,
    pub is_pinned: bool,
    pub created_at: String,
    pub updated_at: String,
    pub published_at: Option<String>,
}

/// 文章 API 响应模型
///
/// 在 [`Post`] 基础上增加：
/// - `html_content`：Markdown 渲染后的 HTML 内容
/// - `author_name`：作者用户名
/// - `category_name`：所属分类名称
/// - `tags`：关联标签列表
#[derive(Debug, Serialize, Clone)]
#[non_exhaustive]
pub struct PostResponse {
    pub id: String,
    pub title: String,
    pub slug: String,
    pub content: String,
    pub html_content: String,
    pub excerpt: Option<String>,
    pub cover_image: Option<String>,
    pub status: String,
    pub author_id: String,
    pub author_name: Option<String>,
    pub category_id: Option<String>,
    pub category_name: Option<String>,
    pub tags: Vec<TagBrief>,
    pub view_count: i64,
    pub is_pinned: bool,
    pub created_at: String,
    pub updated_at: String,
    pub published_at: Option<String>,
}

/// 标签摘要
///
/// 用于文章响应中展示标签的简要信息，包含 ID、名称和 slug。
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TagBrief {
    pub id: String,
    pub name: String,
    pub slug: String,
}

/// 创建文章请求体
///
/// - `title` 长度 1–200 个字符
/// - `content` 不能为空
/// - `status` 默认为 `draft`
/// - `tag_ids` 可选，指定关联标签
#[derive(Debug, Clone, Deserialize, Serialize, Validate)]
pub struct CreatePostRequest {
    #[validate(length(min = 1, max = 200))]
    pub title: String,
    #[validate(length(min = 1))]
    pub content: String,
    pub excerpt: Option<String>,
    pub cover_image: Option<String>,
    pub status: Option<String>,
    pub category_id: Option<String>,
    pub tag_ids: Option<Vec<String>>,
}

/// 更新文章请求体
///
/// 所有字段均为可选，仅更新提供的字段。
/// - `title` 如果提供，长度须在 1–200 个字符之间
#[derive(Debug, Deserialize, Serialize, Validate, Clone)]
pub struct UpdatePostRequest {
    #[validate(length(min = 1, max = 200))]
    pub title: Option<String>,
    pub content: Option<String>,
    pub excerpt: Option<String>,
    pub cover_image: Option<String>,
    pub status: Option<String>,
    pub category_id: Option<String>,
    pub tag_ids: Option<Vec<String>>,
}

/// 根据 slug 查找文章
///
/// 返回 `Ok(Some(post))` 或 `Ok(None)`（未找到时）。
pub async fn find_by_slug(pool: &crate::db::Pool, slug: &str) -> AppResult<Option<Post>> {
    let sql = crate::db::dialect::translate("SELECT * FROM posts WHERE slug = ?");
    let post = sqlx::query_as::<_, Post>(&sql)
        .bind(slug)
        .fetch_optional(pool)
        .await?;
    Ok(post)
}

/// 根据文章 ID 查找文章
///
/// 返回 `Ok(Some(post))` 或 `Ok(None)`（未找到时）。
pub async fn find_by_id(pool: &crate::db::Pool, id: &str) -> AppResult<Option<Post>> {
    let sql = crate::db::dialect::translate("SELECT * FROM posts WHERE id = ?");
    let post = sqlx::query_as::<_, Post>(&sql)
        .bind(id)
        .fetch_optional(pool)
        .await?;
    Ok(post)
}

/// 创建新文章
///
/// 自动生成 UUID v7 作为主键；若 `status` 为 `published` 则同时设置 `published_at`。
/// 创建完成后重新查询并返回完整文章记录。
#[allow(clippy::too_many_arguments)]
pub async fn create(
    pool: &crate::db::Pool,
    title: &str,
    slug: &str,
    content: &str,
    excerpt: Option<&str>,
    cover_image: Option<&str>,
    status: &str,
    author_id: &str,
    category_id: Option<&str>,
) -> AppResult<Post> {
    let id = Uuid::now_v7().to_string();
    let now = Utc::now().to_rfc3339();
    let published_at = if status == "published" {
        Some(now.clone())
    } else {
        None
    };

    sqlx::query!(
        "INSERT INTO posts (id, title, slug, content, excerpt, cover_image, status, author_id, category_id, published_at, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        id,
        title,
        slug,
        content,
        excerpt,
        cover_image,
        status,
        author_id,
        category_id,
        published_at,
        now,
        now,
    )
    .execute(pool)
    .await?;

    find_by_id(pool, &id)
        .await?
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("failed to fetch created post")))
}

/// 更新文章
///
/// 仅更新传入的非空字段，其余保留原值。
/// 若文章首次从草稿变为已发布状态，自动填充 `published_at`。
#[allow(clippy::too_many_arguments)]
pub async fn update(
    pool: &crate::db::Pool,
    id: &str,
    title: Option<&str>,
    slug: Option<&str>,
    content: Option<&str>,
    excerpt: Option<&str>,
    cover_image: Option<&str>,
    status: Option<&str>,
    category_id: Option<&str>,
) -> AppResult<Post> {
    let existing = find_by_id(pool, id)
        .await?
        .ok_or_else(|| AppError::NotFound("post".into()))?;

    let now = Utc::now().to_rfc3339();
    let new_status = status.unwrap_or(&existing.status);
    let published_at = if new_status == "published" && existing.published_at.is_none() {
        Some(now.clone())
    } else {
        existing.published_at
    };

    let title = title.unwrap_or(&existing.title);
    let content = content.unwrap_or(&existing.content);
    let excerpt = excerpt.map(|s| s.to_string()).or(existing.excerpt);
    let cover_image = cover_image.map(|s| s.to_string()).or(existing.cover_image);
    let category_id = category_id.map(|s| s.to_string()).or(existing.category_id);
    let slug = slug.unwrap_or(&existing.slug);

    sqlx::query!(
        "UPDATE posts SET title = ?, slug = ?, content = ?, excerpt = ?, cover_image = ?, status = ?, category_id = ?, published_at = ?, updated_at = ? WHERE id = ?",
        title,
        slug,
        content,
        excerpt,
        cover_image,
        new_status,
        category_id,
        published_at,
        now,
        id,
    )
    .execute(pool)
    .await?;

    find_by_id(pool, id)
        .await?
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("failed to fetch updated post")))
}

/// 删除文章
///
/// 若文章不存在则返回 [`AppError::NotFound`]。
pub async fn delete(pool: &crate::db::Pool, id: &str) -> AppResult<()> {
    let result = sqlx::query!("DELETE FROM posts WHERE id = ?", id)
        .execute(pool)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("post".into()));
    }
    Ok(())
}

/// 原子性地增加文章浏览量并返回 JOIN 查询结果。
///
/// 单条 SQL 完成 UPDATE + SELECT，避免 get_post 中查询与更新之间的竞态。
pub async fn increment_view_count_joined(
    pool: &crate::db::Pool,
    slug: &str,
) -> AppResult<PostJoinedRow> {
    sqlx::query!(
        "UPDATE posts SET view_count = view_count + 1 WHERE slug = ? AND status = 'published'",
        slug
    )
    .execute(pool)
    .await?;

    find_published_joined_by_slug(pool, slug).await
}

/// 同步文章与标签的关联关系
///
/// 在事务中执行：先删除该文章的所有现有关联，再逐条插入新的关联。
pub async fn sync_tags(pool: &crate::db::Pool, post_id: &str, tag_ids: &[String]) -> AppResult<()> {
    let mut tx = pool.begin().await?;

    sqlx::query!("DELETE FROM posts_tags WHERE post_id = ?", post_id)
        .execute(&mut *tx)
        .await?;

    for tag_id in tag_ids {
        sqlx::query!(
            "INSERT INTO posts_tags (post_id, tag_id) VALUES (?, ?)",
            post_id,
            tag_id,
        )
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(())
}

/// 标签查询中间行类型
///
/// 用于从 `tags` 表与 `posts_tags` 关联查询中提取标签的 id、name、slug。
#[derive(Debug, FromRow)]
pub struct TagRow {
    pub id: String,
    pub name: String,
    pub slug: String,
}

/// 获取文章关联的标签列表
///
/// 通过 `posts_tags` 关联表查询，返回 [`TagBrief`] 列表。
pub async fn get_post_tags(pool: &crate::db::Pool, post_id: &str) -> AppResult<Vec<TagBrief>> {
    let sql = crate::db::dialect::translate(
        "SELECT t.id, t.name, t.slug FROM tags t INNER JOIN posts_tags pt ON t.id = pt.tag_id WHERE pt.post_id = ?",
    );
    let rows = sqlx::query_as::<_, TagRow>(&sql)
        .bind(post_id)
        .fetch_all(pool)
        .await?;

    Ok(rows
        .into_iter()
        .map(|r| TagBrief {
            id: r.id,
            name: r.name,
            slug: r.slug,
        })
        .collect())
}

/// 作者名查询中间行类型
///
/// 用于从 `users` 表查询作者用户名。
#[derive(Debug, FromRow)]
pub struct AuthorRow {
    pub username: String,
}

/// 根据用户 ID 获取作者用户名
///
/// 返回 `Ok(Some(username))` 或 `Ok(None)`（用户不存在时）。
pub async fn get_author_name(pool: &crate::db::Pool, author_id: &str) -> AppResult<Option<String>> {
    let sql = crate::db::dialect::translate("SELECT username FROM users WHERE id = ?");
    let row = sqlx::query_as::<_, AuthorRow>(&sql)
        .bind(author_id)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|r| r.username))
}

/// 分类名查询中间行类型
///
/// 用于从 `categories` 表查询分类名称。
#[derive(Debug, FromRow)]
pub struct CategoryNameRow {
    pub name: String,
}

/// 根据分类 ID 获取分类名称
///
/// 返回 `Ok(Some(name))` 或 `Ok(None)`（分类不存在时）。
pub async fn get_category_name(
    pool: &crate::db::Pool,
    category_id: &str,
) -> AppResult<Option<String>> {
    let sql = crate::db::dialect::translate("SELECT name FROM categories WHERE id = ?");
    let row = sqlx::query_as::<_, CategoryNameRow>(&sql)
        .bind(category_id)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|r| r.name))
}

/// 分页查询已发布文章
///
/// 支持按分类 ID、标签 ID、关键词（搜索标题和内容）筛选。
/// 结果按 `is_pinned DESC, created_at DESC` 排序。
/// 返回文章列表和总记录数。
pub async fn find_published(
    pool: &crate::db::Pool,
    page: i64,
    page_size: i64,
    category_id: Option<&str>,
    tag_id: Option<&str>,
    q: Option<&str>,
) -> AppResult<(Vec<Post>, i64)> {
    let offset = (page - 1) * page_size;

    let (posts, total) = if let Some(tag_id) = tag_id {
        let sql = crate::db::dialect::translate(
            "SELECT p.* FROM posts p INNER JOIN posts_tags pt ON p.id = pt.post_id WHERE p.status = 'published' AND pt.tag_id = ? ORDER BY p.is_pinned DESC, p.created_at DESC LIMIT ? OFFSET ?",
        );
        let posts = sqlx::query_as::<_, Post>(&sql)
            .bind(tag_id)
            .bind(page_size)
            .bind(offset)
            .fetch_all(pool)
            .await?;

        let sql = crate::db::dialect::translate(
            "SELECT COUNT(*) FROM posts p INNER JOIN posts_tags pt ON p.id = pt.post_id WHERE p.status = 'published' AND pt.tag_id = ?",
        );
        let total: (i64,) = sqlx::query_as(&sql).bind(tag_id).fetch_one(pool).await?;

        (posts, total.0)
    } else if let Some(q) = q {
        let pattern = format!("%{}%", q);
        let sql = crate::db::dialect::translate(
            "SELECT * FROM posts WHERE status = 'published' AND (title LIKE ? OR content LIKE ?) ORDER BY is_pinned DESC, created_at DESC LIMIT ? OFFSET ?",
        );
        let posts = sqlx::query_as::<_, Post>(&sql)
            .bind(&pattern)
            .bind(&pattern)
            .bind(page_size)
            .bind(offset)
            .fetch_all(pool)
            .await?;

        let sql = crate::db::dialect::translate(
            "SELECT COUNT(*) FROM posts WHERE status = 'published' AND (title LIKE ? OR content LIKE ?)",
        );
        let total: (i64,) = sqlx::query_as(&sql)
            .bind(&pattern)
            .bind(&pattern)
            .fetch_one(pool)
            .await?;

        (posts, total.0)
    } else if let Some(category_id) = category_id {
        let sql = crate::db::dialect::translate(
            "SELECT * FROM posts WHERE status = 'published' AND category_id = ? ORDER BY is_pinned DESC, created_at DESC LIMIT ? OFFSET ?",
        );
        let posts = sqlx::query_as::<_, Post>(&sql)
            .bind(category_id)
            .bind(page_size)
            .bind(offset)
            .fetch_all(pool)
            .await?;

        let sql = crate::db::dialect::translate(
            "SELECT COUNT(*) FROM posts WHERE status = 'published' AND category_id = ?",
        );
        let total: (i64,) = sqlx::query_as(&sql)
            .bind(category_id)
            .fetch_one(pool)
            .await?;

        (posts, total.0)
    } else {
        let sql = crate::db::dialect::translate(
            "SELECT * FROM posts WHERE status = 'published' ORDER BY is_pinned DESC, created_at DESC LIMIT ? OFFSET ?",
        );
        let posts = sqlx::query_as::<_, Post>(&sql)
            .bind(page_size)
            .bind(offset)
            .fetch_all(pool)
            .await?;

        let total: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM posts WHERE status = 'published'")
            .fetch_one(pool)
            .await?;

        (posts, total.0)
    };

    Ok((posts, total))
}

/// 根据 slug 查找已发布文章
///
/// 仅返回状态为 `published` 的文章；若未找到则返回 [`AppError::NotFound`]。
pub async fn find_published_by_slug(pool: &crate::db::Pool, slug: &str) -> AppResult<Post> {
    let sql = crate::db::dialect::translate(
        "SELECT * FROM posts WHERE slug = ? AND status = 'published'",
    );
    sqlx::query_as::<_, Post>(&sql)
        .bind(slug)
        .fetch_one(pool)
        .await
        .map_err(Into::into)
}

/// JOIN 查询中间行类型（含作者名和分类名）
#[derive(Debug, FromRow)]
pub struct PostJoinedRow {
    pub id: String,
    pub title: String,
    pub slug: String,
    pub content: String,
    pub excerpt: Option<String>,
    pub cover_image: Option<String>,
    pub status: String,
    pub author_id: String,
    pub category_id: Option<String>,
    pub view_count: i64,
    pub is_pinned: bool,
    pub created_at: String,
    pub updated_at: String,
    pub published_at: Option<String>,
    pub author_name: Option<String>,
    pub category_name: Option<String>,
}

const JOIN_SQL: &str = "\
    SELECT p.id, p.title, p.slug, p.content, p.excerpt, p.cover_image, p.status, \
    p.author_id, p.category_id, p.view_count, p.is_pinned, p.created_at, p.updated_at, \
    p.published_at, u.username AS author_name, c.name AS category_name \
    FROM posts p \
    LEFT JOIN users u ON p.author_id = u.id \
    LEFT JOIN categories c ON p.category_id = c.id";

/// 根据 ID 用 JOIN 查询单篇文章（含作者名和分类名）
pub async fn find_joined_by_id(pool: &crate::db::Pool, id: &str) -> AppResult<PostJoinedRow> {
    let sql = format!("{} WHERE p.id = ?", JOIN_SQL);
    let sql = crate::db::dialect::translate(&sql);
    sqlx::query_as::<_, PostJoinedRow>(&sql)
        .bind(id)
        .fetch_one(pool)
        .await
        .map_err(Into::into)
}

/// 根据 slug 用 JOIN 查询已发布单篇文章（含作者名和分类名）
pub async fn find_published_joined_by_slug(
    pool: &crate::db::Pool,
    slug: &str,
) -> AppResult<PostJoinedRow> {
    let sql = format!("{} WHERE p.slug = ? AND p.status = 'published'", JOIN_SQL);
    let sql = crate::db::dialect::translate(&sql);
    sqlx::query_as::<_, PostJoinedRow>(&sql)
        .bind(slug)
        .fetch_one(pool)
        .await
        .map_err(Into::into)
}

/// 批量获取多篇文章的标签
///
/// 返回以 `post_id` 为键的 `HashMap`，每个值是该文章的标签列表。
pub async fn get_tags_for_posts(
    pool: &crate::db::Pool,
    post_ids: &[String],
) -> AppResult<std::collections::HashMap<String, Vec<TagBrief>>> {
    if post_ids.is_empty() {
        return Ok(std::collections::HashMap::new());
    }

    let placeholders: Vec<&str> = post_ids.iter().map(|_| "?").collect();
    let sql = format!(
        "SELECT pt.post_id, t.id, t.name, t.slug \
         FROM posts_tags pt \
         JOIN tags t ON pt.tag_id = t.id \
         WHERE pt.post_id IN ({})",
        placeholders.join(",")
    );

    #[derive(Debug, FromRow)]
    struct TagWithPostId {
        post_id: String,
        id: String,
        name: String,
        slug: String,
    }

    let translated = crate::db::dialect::translate(&sql);
    let mut query = sqlx::query_as::<_, TagWithPostId>(&translated);
    for id in post_ids {
        query = query.bind(id);
    }
    let rows = query.fetch_all(pool).await?;

    let mut map: std::collections::HashMap<String, Vec<TagBrief>> =
        std::collections::HashMap::new();
    for row in rows {
        map.entry(row.post_id).or_default().push(TagBrief {
            id: row.id,
            name: row.name,
            slug: row.slug,
        });
    }
    Ok(map)
}

/// 分页查询已发布文章（JOIN 用户和分类表）
///
/// 与 [`find_published`] 相同的筛选逻辑，但通过 LEFT JOIN 一次性获取
/// `author_name` 和 `category_name`，避免 N+1 查询。
pub async fn find_published_joined(
    pool: &crate::db::Pool,
    page: i64,
    page_size: i64,
    category_id: Option<&str>,
    tag_id: Option<&str>,
    q: Option<&str>,
) -> AppResult<(Vec<PostJoinedRow>, i64)> {
    let offset = (page - 1) * page_size;

    let base_select = "\
        SELECT p.id, p.title, p.slug, p.content, p.excerpt, p.cover_image, p.status, \
        p.author_id, p.category_id, p.view_count, p.is_pinned, p.created_at, p.updated_at, \
        p.published_at, u.username AS author_name, c.name AS category_name \
        FROM posts p \
        LEFT JOIN users u ON p.author_id = u.id \
        LEFT JOIN categories c ON p.category_id = c.id";

    let (posts, total) = if let Some(tag_id) = tag_id {
        let sql = format!(
            "{} \
             INNER JOIN posts_tags pt ON p.id = pt.post_id \
             WHERE p.status = 'published' AND pt.tag_id = ? \
             ORDER BY p.is_pinned DESC, p.created_at DESC LIMIT ? OFFSET ?",
            base_select
        );
        let sql = crate::db::dialect::translate(&sql);
        let posts = sqlx::query_as::<_, PostJoinedRow>(&sql)
            .bind(tag_id)
            .bind(page_size)
            .bind(offset)
            .fetch_all(pool)
            .await?;

        let sql = crate::db::dialect::translate(
            "SELECT COUNT(*) FROM posts p INNER JOIN posts_tags pt ON p.id = pt.post_id WHERE p.status = 'published' AND pt.tag_id = ?",
        );
        let total: (i64,) = sqlx::query_as(&sql).bind(tag_id).fetch_one(pool).await?;

        (posts, total.0)
    } else if let Some(q) = q {
        let pattern = format!("%{}%", q);
        let sql = format!(
            "{} \
             WHERE p.status = 'published' AND (p.title LIKE ? OR p.content LIKE ?) \
             ORDER BY p.is_pinned DESC, p.created_at DESC LIMIT ? OFFSET ?",
            base_select
        );
        let sql = crate::db::dialect::translate(&sql);
        let posts = sqlx::query_as::<_, PostJoinedRow>(&sql)
            .bind(&pattern)
            .bind(&pattern)
            .bind(page_size)
            .bind(offset)
            .fetch_all(pool)
            .await?;

        let sql = crate::db::dialect::translate(
            "SELECT COUNT(*) FROM posts WHERE status = 'published' AND (title LIKE ? OR content LIKE ?)",
        );
        let total: (i64,) = sqlx::query_as(&sql)
            .bind(&pattern)
            .bind(&pattern)
            .fetch_one(pool)
            .await?;

        (posts, total.0)
    } else if let Some(category_id) = category_id {
        let sql = format!(
            "{} \
             WHERE p.status = 'published' AND p.category_id = ? \
             ORDER BY p.is_pinned DESC, p.created_at DESC LIMIT ? OFFSET ?",
            base_select
        );
        let sql = crate::db::dialect::translate(&sql);
        let posts = sqlx::query_as::<_, PostJoinedRow>(&sql)
            .bind(category_id)
            .bind(page_size)
            .bind(offset)
            .fetch_all(pool)
            .await?;

        let sql = crate::db::dialect::translate(
            "SELECT COUNT(*) FROM posts WHERE status = 'published' AND category_id = ?",
        );
        let total: (i64,) = sqlx::query_as(&sql)
            .bind(category_id)
            .fetch_one(pool)
            .await?;

        (posts, total.0)
    } else {
        let sql = format!(
            "{} \
             WHERE p.status = 'published' \
             ORDER BY p.is_pinned DESC, p.created_at DESC LIMIT ? OFFSET ?",
            base_select
        );
        let sql = crate::db::dialect::translate(&sql);
        let posts = sqlx::query_as::<_, PostJoinedRow>(&sql)
            .bind(page_size)
            .bind(offset)
            .fetch_all(pool)
            .await?;

        let total: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM posts WHERE status = 'published'")
            .fetch_one(pool)
            .await?;

        (posts, total.0)
    };

    Ok((posts, total))
}
