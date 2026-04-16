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

use crate::db::tenant::{resolve_tenant, tenant_filter, tenant_filter_aliased};
use crate::errors::app_error::{AppError, AppResult};

/// 文章完整数据库行模型
///
/// 直接映射 `posts` 表的所有字段。
/// 首次发布时自动填充 `published_at`；`status` 可取 `draft`、`published` 等。
#[derive(Debug, FromRow, Serialize, Deserialize, Clone)]
#[non_exhaustive]
pub struct Post {
    pub id: String,
    pub tenant_id: String,
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

/// 标签摘要
///
/// 用于文章响应中展示标签的简要信息，包含 ID、名称和 slug。
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TagBrief {
    pub id: String,
    pub name: String,
    pub slug: String,
}

/// 根据 slug 查找文章
///
/// 返回 `Ok(Some(post))` 或 `Ok(None)`（未找到时）。
pub async fn find_by_slug(
    pool: &crate::db::Pool,
    slug: &str,
    tenant_id: Option<&str>,
) -> AppResult<Option<Post>> {
    let sql = format!(
        "SELECT * FROM posts WHERE slug = ?{}",
        tenant_filter(tenant_id)
    );
    let sql = crate::db::dialect::translate(&sql);
    let mut q = sqlx::query_as::<_, Post>(&sql).bind(slug);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    let post = q.fetch_optional(pool).await?;
    Ok(post)
}

/// 根据文章 ID 查找文章
///
/// 返回 `Ok(Some(post))` 或 `Ok(None)`（未找到时）。
pub async fn find_by_id(
    pool: &crate::db::Pool,
    id: &str,
    tenant_id: Option<&str>,
) -> AppResult<Option<Post>> {
    let sql = format!(
        "SELECT * FROM posts WHERE id = ?{}",
        tenant_filter(tenant_id)
    );
    let sql = crate::db::dialect::translate(&sql);
    let mut q = sqlx::query_as::<_, Post>(&sql).bind(id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    let post = q.fetch_optional(pool).await?;
    Ok(post)
}

/// 创建新文章
///
/// 自动生成 UUID v7 作为主键；若 `status` 为 `published` 则同时设置 `published_at`。
/// 创建完成后重新查询并返回完整文章记录。
pub async fn create(
    pool: &crate::db::Pool,
    cmd: &crate::commands::CreatePostCmd,
    tenant_id: Option<&str>,
) -> AppResult<Post> {
    let mut tx = pool.begin().await?;
    let post = create_tx(&mut tx, cmd, tenant_id).await?;
    tx.commit().await?;
    Ok(post)
}

/// 在已有事务中创建新文章
pub async fn create_tx(
    tx: &mut crate::db::Transaction<'_>,
    cmd: &crate::commands::CreatePostCmd,
    tenant_id: Option<&str>,
) -> AppResult<Post> {
    let (id, now) = crate::utils::id::new_id_and_timestamp();
    let published_at = if cmd.status == "published" {
        Some(now.clone())
    } else {
        None
    };
    let tid = resolve_tenant(tenant_id);

    let sql = crate::db::dialect::translate(
        "INSERT INTO posts (id, tenant_id, title, slug, content, excerpt, cover_image, status, author_id, category_id, published_at, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    );
    sqlx::query(&sql)
        .bind(&id)
        .bind(tid)
        .bind(&cmd.title)
        .bind(&cmd.slug)
        .bind(&cmd.content)
        .bind(&cmd.excerpt)
        .bind(&cmd.cover_image)
        .bind(&cmd.status)
        .bind(&cmd.author_id)
        .bind(&cmd.category_id)
        .bind(&published_at)
        .bind(&now)
        .bind(&now)
        .execute(&mut **tx)
        .await?;

    let sql = format!(
        "SELECT * FROM posts WHERE id = ?{}",
        tenant_filter(tenant_id)
    );
    let sql = crate::db::dialect::translate(&sql);
    let mut q = sqlx::query_as::<_, Post>(&sql).bind(&id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    let post = q
        .fetch_one(&mut **tx)
        .await
        .map_err(|_| AppError::Internal(anyhow::anyhow!("failed to fetch created post")))?;
    Ok(post)
}

/// 更新文章
///
/// 仅更新传入的非空字段，其余保留原值。
/// 若文章首次从草稿变为已发布状态，自动填充 `published_at`。
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

/// 在已有事务中更新文章
pub async fn update_tx(
    tx: &mut crate::db::Transaction<'_>,
    cmd: &crate::commands::UpdatePostCmd,
    tenant_id: Option<&str>,
) -> AppResult<Post> {
    let sql = format!(
        "SELECT * FROM posts WHERE id = ?{}",
        tenant_filter(tenant_id)
    );
    let sql = crate::db::dialect::translate(&sql);
    let mut q = sqlx::query_as::<_, Post>(&sql).bind(&cmd.id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    let existing = q
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(|| AppError::not_found("post"))?;

    let now = Utc::now().to_rfc3339();
    let new_status = cmd.status.as_deref().unwrap_or(&existing.status);
    let published_at = if new_status == "published" && existing.published_at.is_none() {
        Some(now.clone())
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
    let category_id = cmd
        .category_id
        .as_deref()
        .map(std::string::ToString::to_string)
        .or(existing.category_id);
    let slug = cmd.slug.as_deref().unwrap_or(&existing.slug);

    let sql = format!(
        "UPDATE posts SET title = ?, slug = ?, content = ?, excerpt = ?, cover_image = ?, status = ?, category_id = ?, published_at = ?, updated_at = ? WHERE id = ?{}",
        tenant_filter(tenant_id)
    );
    let sql = crate::db::dialect::translate(&sql);
    let mut q = sqlx::query(&sql)
        .bind(title)
        .bind(slug)
        .bind(content)
        .bind(excerpt)
        .bind(cover_image)
        .bind(new_status)
        .bind(category_id)
        .bind(published_at)
        .bind(now)
        .bind(&cmd.id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    q.execute(&mut **tx).await?;

    let sql = format!(
        "SELECT * FROM posts WHERE id = ?{}",
        tenant_filter(tenant_id)
    );
    let sql = crate::db::dialect::translate(&sql);
    let mut q = sqlx::query_as::<_, Post>(&sql).bind(&cmd.id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    q.fetch_one(&mut **tx)
        .await
        .map_err(|_| AppError::Internal(anyhow::anyhow!("failed to fetch updated post")))
}

/// 删除文章
///
/// 若文章不存在则返回 [`AppError::NotFound`]。
pub async fn delete(pool: &crate::db::Pool, id: &str, tenant_id: Option<&str>) -> AppResult<()> {
    let sql = format!("DELETE FROM posts WHERE id = ?{}", tenant_filter(tenant_id));
    let sql = crate::db::dialect::translate(&sql);
    let mut q = sqlx::query(&sql).bind(id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    let result = q.execute(pool).await?;

    AppError::expect_affected(&result, "post")
}

/// 原子性地增加文章浏览量并返回 JOIN 查询结果。
///
/// 单条 SQL 完成 UPDATE + SELECT，避免 `get_post` 中查询与更新之间的竞态。
pub async fn increment_view_count_joined(
    pool: &crate::db::Pool,
    slug: &str,
    tenant_id: Option<&str>,
) -> AppResult<PostJoinedRow> {
    let sql = format!(
        "UPDATE posts SET view_count = view_count + 1 WHERE slug = ? AND status = 'published'{}",
        tenant_filter(tenant_id)
    );
    let sql = crate::db::dialect::translate(&sql);
    let mut q = sqlx::query(&sql).bind(slug);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    q.execute(pool).await?;

    find_published_joined_by_slug(pool, slug, tenant_id).await
}

/// 同步文章与标签的关联关系
///
/// 在事务中执行：先删除该文章的所有现有关联，再逐条插入新的关联。
pub async fn sync_tags(pool: &crate::db::Pool, post_id: &str, tag_ids: &[String]) -> AppResult<()> {
    let mut tx = pool.begin().await?;
    sync_tags_tx(&mut tx, post_id, tag_ids).await?;
    tx.commit().await?;
    Ok(())
}

/// 在已有事务中同步文章与标签的关联关系
pub async fn sync_tags_tx(
    tx: &mut crate::db::Transaction<'_>,
    post_id: &str,
    tag_ids: &[String],
) -> AppResult<()> {
    sqlx::query!("DELETE FROM posts_tags WHERE post_id = ?", post_id)
        .execute(&mut **tx)
        .await?;

    for tag_id in tag_ids {
        sqlx::query!(
            "INSERT INTO posts_tags (post_id, tag_id) VALUES (?, ?)",
            post_id,
            tag_id,
        )
        .execute(&mut **tx)
        .await?;
    }

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
pub async fn get_post_tags(
    pool: &crate::db::Pool,
    post_id: &str,
    tenant_id: Option<&str>,
) -> AppResult<Vec<TagBrief>> {
    let sql = format!(
        "SELECT t.id, t.name, t.slug FROM tags t INNER JOIN posts_tags pt ON t.id = pt.tag_id WHERE pt.post_id = ?{}",
        tenant_filter_aliased("t", tenant_id)
    );
    let sql = crate::db::dialect::translate(&sql);
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
pub async fn get_author_name(
    pool: &crate::db::Pool,
    author_id: &str,
    tenant_id: Option<&str>,
) -> AppResult<Option<String>> {
    let sql = format!(
        "SELECT username FROM users WHERE id = ?{}",
        tenant_filter(tenant_id)
    );
    let sql = crate::db::dialect::translate(&sql);
    let mut q = sqlx::query_as::<_, AuthorRow>(&sql).bind(author_id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    let row = q.fetch_optional(pool).await?;
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
    tenant_id: Option<&str>,
) -> AppResult<Option<String>> {
    let sql = format!(
        "SELECT name FROM categories WHERE id = ?{}",
        tenant_filter(tenant_id)
    );
    let sql = crate::db::dialect::translate(&sql);
    let mut q = sqlx::query_as::<_, CategoryNameRow>(&sql).bind(category_id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    let row = q.fetch_optional(pool).await?;
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
    tenant_id: Option<&str>,
) -> AppResult<(Vec<Post>, i64)> {
    let offset = (page - 1) * page_size;
    let filter = tenant_filter_aliased("p", tenant_id);

    let (posts, total) = if let Some(tag_id) = tag_id {
        let sql = format!(
            "SELECT p.* FROM posts p INNER JOIN posts_tags pt ON p.id = pt.post_id WHERE p.status = 'published' AND pt.tag_id = ?{filter} ORDER BY p.is_pinned DESC, p.created_at DESC LIMIT ? OFFSET ?"
        );
        let sql = crate::db::dialect::translate(&sql);
        let mut query = sqlx::query_as::<_, Post>(&sql).bind(tag_id);
        if let Some(tid) = tenant_id {
            query = query.bind(tid);
        }
        let posts = query.bind(page_size).bind(offset).fetch_all(pool).await?;

        let filter = tenant_filter_aliased("p", tenant_id);
        let sql = format!(
            "SELECT COUNT(*) FROM posts p INNER JOIN posts_tags pt ON p.id = pt.post_id WHERE p.status = 'published' AND pt.tag_id = ?{filter}"
        );
        let sql = crate::db::dialect::translate(&sql);
        let mut query = sqlx::query_as::<_, (i64,)>(&sql).bind(tag_id);
        if let Some(tid) = tenant_id {
            query = query.bind(tid);
        }
        let total = query.fetch_one(pool).await?;

        (posts, total.0)
    } else if let Some(q) = q {
        let pattern = format!("%{q}%");
        let filter = tenant_filter(tenant_id);
        let sql = format!(
            "SELECT * FROM posts WHERE status = 'published'{filter} AND (title LIKE ? OR content LIKE ?) ORDER BY is_pinned DESC, created_at DESC LIMIT ? OFFSET ?"
        );
        let sql = crate::db::dialect::translate(&sql);
        let mut query = sqlx::query_as::<_, Post>(&sql);
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
            "SELECT COUNT(*) FROM posts WHERE status = 'published'{filter} AND (title LIKE ? OR content LIKE ?)"
        );
        let sql = crate::db::dialect::translate(&sql);
        let mut query = sqlx::query_as::<_, (i64,)>(&sql);
        if let Some(tid) = tenant_id {
            query = query.bind(tid);
        }
        let total = query.bind(&pattern).bind(&pattern).fetch_one(pool).await?;

        (posts, total.0)
    } else if let Some(category_id) = category_id {
        let filter = tenant_filter(tenant_id);
        let sql = format!(
            "SELECT * FROM posts WHERE status = 'published' AND category_id = ?{filter} ORDER BY is_pinned DESC, created_at DESC LIMIT ? OFFSET ?"
        );
        let sql = crate::db::dialect::translate(&sql);
        let mut query = sqlx::query_as::<_, Post>(&sql).bind(category_id);
        if let Some(tid) = tenant_id {
            query = query.bind(tid);
        }
        let posts = query.bind(page_size).bind(offset).fetch_all(pool).await?;

        let sql = format!(
            "SELECT COUNT(*) FROM posts WHERE status = 'published' AND category_id = ?{filter}"
        );
        let sql = crate::db::dialect::translate(&sql);
        let mut query = sqlx::query_as::<_, (i64,)>(&sql).bind(category_id);
        if let Some(tid) = tenant_id {
            query = query.bind(tid);
        }
        let total = query.fetch_one(pool).await?;

        (posts, total.0)
    } else {
        let filter = tenant_filter(tenant_id);
        let sql = format!(
            "SELECT * FROM posts WHERE status = 'published'{filter} ORDER BY is_pinned DESC, created_at DESC LIMIT ? OFFSET ?"
        );
        let sql = crate::db::dialect::translate(&sql);
        let mut query = sqlx::query_as::<_, Post>(&sql);
        if let Some(tid) = tenant_id {
            query = query.bind(tid);
        }
        let posts = query.bind(page_size).bind(offset).fetch_all(pool).await?;

        let sql = format!("SELECT COUNT(*) FROM posts WHERE status = 'published'{filter}");
        let sql = crate::db::dialect::translate(&sql);
        let mut query = sqlx::query_as::<_, (i64,)>(&sql);
        if let Some(tid) = tenant_id {
            query = query.bind(tid);
        }
        let total = query.fetch_one(pool).await?;

        (posts, total.0)
    };

    Ok((posts, total))
}

/// 查询全部文章（包含所有状态），用于后台管理
///
/// 支持按 `status` 筛选，`None` 表示返回全部状态。
/// 通过 LEFT JOIN 获取 `author_name` 和 `category_name`。
pub async fn find_all_joined(
    pool: &crate::db::Pool,
    page: i64,
    page_size: i64,
    status: Option<&str>,
    tenant_id: Option<&str>,
) -> AppResult<(Vec<PostJoinedRow>, i64)> {
    let offset = (page - 1) * page_size;

    let (posts, total) = if let Some(status) = status {
        let filter = tenant_filter_aliased("p", tenant_id);
        let sql = format!(
            "{JOIN_SQL} \
             WHERE p.status = ?{filter} \
             ORDER BY p.is_pinned DESC, p.created_at DESC LIMIT ? OFFSET ?"
        );
        let sql = crate::db::dialect::translate(&sql);
        let mut query = sqlx::query_as::<_, PostJoinedRow>(&sql).bind(status);
        if let Some(tid) = tenant_id {
            query = query.bind(tid);
        }
        let posts = query.bind(page_size).bind(offset).fetch_all(pool).await?;
        let filter = tenant_filter(tenant_id);
        let sql = format!("SELECT COUNT(*) FROM posts WHERE status = ?{filter}");
        let sql = crate::db::dialect::translate(&sql);
        let mut query = sqlx::query_as::<_, (i64,)>(&sql).bind(status);
        if let Some(tid) = tenant_id {
            query = query.bind(tid);
        }
        let total = query.fetch_one(pool).await?;
        (posts, total.0)
    } else {
        let filter = tenant_filter_aliased("p", tenant_id);
        let sql = format!(
            "{JOIN_SQL} \
             WHERE 1=1{filter} \
             ORDER BY p.is_pinned DESC, p.created_at DESC LIMIT ? OFFSET ?"
        );
        let sql = crate::db::dialect::translate(&sql);
        let mut query = sqlx::query_as::<_, PostJoinedRow>(&sql);
        if let Some(tid) = tenant_id {
            query = query.bind(tid);
        }
        let posts = query.bind(page_size).bind(offset).fetch_all(pool).await?;
        let sql = format!(
            "SELECT COUNT(*) FROM posts WHERE 1=1{}",
            tenant_filter(tenant_id)
        );
        let sql = crate::db::dialect::translate(&sql);
        let mut query = sqlx::query_as::<_, (i64,)>(&sql);
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
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(include_str!("../../migrations/001_init.sql"))
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(include_str!("../../migrations/002_add_indexes.sql"))
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(include_str!("../../migrations/009_options.sql"))
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(include_str!("../../migrations/010_rbac.sql"))
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(include_str!("../../migrations/011_tenants.sql"))
            .execute(&pool)
            .await
            .unwrap();
        pool
    }

    async fn create_user(pool: &crate::db::Pool) -> String {
        let uid = uuid::Uuid::now_v7().to_string();
        sqlx::query(
            "INSERT INTO users (id, username, email, password_hash, role) VALUES (?, 'testuser', 'test@test.com', 'hash', 'author')",
        )
        .bind(&uid)
        .execute(pool)
        .await
        .unwrap();
        uid
    }

    async fn create_test_post(
        pool: &crate::db::Pool,
        author_id: &str,
        status: &str,
        title: &str,
    ) -> Post {
        create(
            pool,
            &CreatePostCmd {
                title: title.to_string(),
                slug: title.to_lowercase().replace(' ', "-"),
                content: format!("{title}的内容"),
                excerpt: None,
                cover_image: None,
                status: status.to_string(),
                author_id: author_id.to_string(),
                category_id: None,
                tag_ids: None,
            },
            Some(crate::db::tenant::DEFAULT_TENANT),
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn find_joined_by_ids_empty() {
        let pool = setup_pool().await;
        let result = find_joined_by_ids(&pool, &[], Some(crate::db::tenant::DEFAULT_TENANT))
            .await
            .unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn find_joined_by_ids_single() {
        let pool = setup_pool().await;
        let uid = create_user(&pool).await;
        let p = create_test_post(&pool, &uid, "published", "测试文章").await;
        let result = find_joined_by_ids(
            &pool,
            &[p.id.clone()],
            Some(crate::db::tenant::DEFAULT_TENANT),
        )
        .await
        .unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, p.id);
        assert_eq!(result[0].title, "测试文章");
        assert_eq!(result[0].author_name.as_deref(), Some("testuser"));
    }

    #[tokio::test]
    async fn find_joined_by_ids_multiple() {
        let pool = setup_pool().await;
        let uid = create_user(&pool).await;
        let p1 = create_test_post(&pool, &uid, "published", "文章A").await;
        let p2 = create_test_post(&pool, &uid, "published", "文章B").await;
        let p3 = create_test_post(&pool, &uid, "published", "文章C").await;
        let result = find_joined_by_ids(
            &pool,
            &[p1.id.clone(), p3.id.clone()],
            Some(crate::db::tenant::DEFAULT_TENANT),
        )
        .await
        .unwrap();
        assert_eq!(result.len(), 2);
        let ids: Vec<&str> = result.iter().map(|r| r.id.as_str()).collect();
        assert!(ids.contains(&p1.id.as_str()));
        assert!(ids.contains(&p3.id.as_str()));
        assert!(!ids.contains(&p2.id.as_str()));
    }

    #[tokio::test]
    async fn find_joined_by_ids_filters_draft() {
        let pool = setup_pool().await;
        let uid = create_user(&pool).await;
        let p = create_test_post(&pool, &uid, "draft", "草稿文章").await;
        let result = find_joined_by_ids(
            &pool,
            &[p.id.clone()],
            Some(crate::db::tenant::DEFAULT_TENANT),
        )
        .await
        .unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn find_joined_by_ids_nonexistent() {
        let pool = setup_pool().await;
        let result = find_joined_by_ids(
            &pool,
            &["nonexistent-id".to_string()],
            Some(crate::db::tenant::DEFAULT_TENANT),
        )
        .await
        .unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn find_joined_by_ids_mixed_published_and_draft() {
        let pool = setup_pool().await;
        let uid = create_user(&pool).await;
        let pub_post = create_test_post(&pool, &uid, "published", "已发布").await;
        let draft_post = create_test_post(&pool, &uid, "draft", "草稿").await;
        let result = find_joined_by_ids(
            &pool,
            &[pub_post.id.clone(), draft_post.id.clone()],
            Some(crate::db::tenant::DEFAULT_TENANT),
        )
        .await
        .unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].title, "已发布");
    }

    #[tokio::test]
    async fn find_joined_by_ids_with_category() {
        let pool = setup_pool().await;
        let uid = create_user(&pool).await;
        let cat_id = uuid::Uuid::now_v7().to_string();
        sqlx::query("INSERT INTO categories (id, name, slug) VALUES (?, '技术', 'tech')")
            .bind(&cat_id)
            .execute(&pool)
            .await
            .unwrap();
        let p = create(
            &pool,
            &CreatePostCmd {
                title: "分类文章".to_string(),
                slug: "cat-post".to_string(),
                content: "内容".to_string(),
                excerpt: None,
                cover_image: None,
                status: "published".to_string(),
                author_id: uid,
                category_id: Some(cat_id),
                tag_ids: None,
            },
            Some(crate::db::tenant::DEFAULT_TENANT),
        )
        .await
        .unwrap();
        let result = find_joined_by_ids(
            &pool,
            &[p.id.clone()],
            Some(crate::db::tenant::DEFAULT_TENANT),
        )
        .await
        .unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].category_name.as_deref(), Some("技术"));
    }

    #[tokio::test]
    async fn count_published_by_ids_empty() {
        let pool = setup_pool().await;
        let count = count_published_by_ids(&pool, &[], Some(crate::db::tenant::DEFAULT_TENANT))
            .await
            .unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn count_published_by_ids_single() {
        let pool = setup_pool().await;
        let uid = create_user(&pool).await;
        let p = create_test_post(&pool, &uid, "published", "计数文章").await;
        let count = count_published_by_ids(&pool, &[p.id], Some(crate::db::tenant::DEFAULT_TENANT))
            .await
            .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn count_published_by_ids_filters_draft() {
        let pool = setup_pool().await;
        let uid = create_user(&pool).await;
        let p = create_test_post(&pool, &uid, "draft", "草稿").await;
        let count = count_published_by_ids(&pool, &[p.id], Some(crate::db::tenant::DEFAULT_TENANT))
            .await
            .unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn count_published_by_ids_multiple() {
        let pool = setup_pool().await;
        let uid = create_user(&pool).await;
        let p1 = create_test_post(&pool, &uid, "published", "A").await;
        let p2 = create_test_post(&pool, &uid, "draft", "B").await;
        let p3 = create_test_post(&pool, &uid, "published", "C").await;
        let count = count_published_by_ids(
            &pool,
            &[p1.id, p2.id, p3.id],
            Some(crate::db::tenant::DEFAULT_TENANT),
        )
        .await
        .unwrap();
        assert_eq!(count, 2);
    }

    #[tokio::test]
    async fn count_published_by_ids_nonexistent() {
        let pool = setup_pool().await;
        let count = count_published_by_ids(
            &pool,
            &["fake-id".to_string()],
            Some(crate::db::tenant::DEFAULT_TENANT),
        )
        .await
        .unwrap();
        assert_eq!(count, 0);
    }
}

/// JOIN 查询中间行类型（含作者名和分类名）
#[derive(Debug, FromRow, Serialize, Deserialize, Clone)]
pub struct PostJoinedRow {
    pub id: String,
    pub tenant_id: String,
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
    SELECT p.id, p.tenant_id, p.title, p.slug, p.content, p.excerpt, p.cover_image, p.status, \
    p.author_id, p.category_id, p.view_count, p.is_pinned, p.created_at, p.updated_at, \
    p.published_at, u.username AS author_name, c.name AS category_name \
    FROM posts p \
    LEFT JOIN users u ON p.author_id = u.id \
    LEFT JOIN categories c ON p.category_id = c.id";

/// 根据 ID 用 JOIN 查询单篇文章（含作者名和分类名）
pub async fn find_joined_by_id(
    pool: &crate::db::Pool,
    id: &str,
    tenant_id: Option<&str>,
) -> AppResult<PostJoinedRow> {
    let sql = format!(
        "{JOIN_SQL} WHERE p.id = ?{}",
        tenant_filter_aliased("p", tenant_id)
    );
    let sql = crate::db::dialect::translate(&sql);
    let mut q = sqlx::query_as::<_, PostJoinedRow>(&sql).bind(id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    q.fetch_one(pool).await.map_err(Into::into)
}

/// 根据 slug 用 JOIN 查询已发布单篇文章（含作者名和分类名）
pub async fn find_published_joined_by_slug(
    pool: &crate::db::Pool,
    slug: &str,
    tenant_id: Option<&str>,
) -> AppResult<PostJoinedRow> {
    let sql = format!(
        "{JOIN_SQL} WHERE p.slug = ? AND p.status = 'published'{}",
        tenant_filter_aliased("p", tenant_id)
    );
    let sql = crate::db::dialect::translate(&sql);
    let mut q = sqlx::query_as::<_, PostJoinedRow>(&sql).bind(slug);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    q.fetch_one(pool).await.map_err(Into::into)
}

/// 批量获取多篇文章的标签
///
/// 返回以 `post_id` 为键的 `HashMap`，每个值是该文章的标签列表。
pub async fn get_tags_for_posts(
    pool: &crate::db::Pool,
    post_ids: &[String],
    tenant_id: Option<&str>,
) -> AppResult<std::collections::HashMap<String, Vec<TagBrief>>> {
    if post_ids.is_empty() {
        return Ok(std::collections::HashMap::new());
    }

    let placeholders: Vec<&str> = post_ids.iter().map(|_| "?").collect();
    let sql = format!(
        "SELECT pt.post_id, t.id, t.name, t.slug \
         FROM posts_tags pt \
         JOIN tags t ON pt.tag_id = t.id \
         WHERE pt.post_id IN ({}){}",
        placeholders.join(","),
        tenant_filter_aliased("t", tenant_id)
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
    if let Some(tid) = tenant_id {
        query = query.bind(tid);
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

/// 根据 ID 列表批量查询已发布文章（JOIN 用户和分类表）
///
/// 用于搜索引擎返回 ID 后从数据库获取完整行数据。
/// 按 `is_pinned DESC, created_at DESC` 排序，结果不超出 `ids` 范围。
pub async fn find_joined_by_ids(
    pool: &crate::db::Pool,
    ids: &[String],
    tenant_id: Option<&str>,
) -> AppResult<Vec<PostJoinedRow>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }

    let placeholders: Vec<&str> = ids.iter().map(|_| "?").collect();
    let sql = format!(
        "{} \
         WHERE p.id IN ({}) AND p.status = 'published'{} \
         ORDER BY p.is_pinned DESC, p.created_at DESC",
        JOIN_SQL,
        placeholders.join(","),
        tenant_filter_aliased("p", tenant_id)
    );

    let translated = crate::db::dialect::translate(&sql);
    let mut query = sqlx::query_as::<_, PostJoinedRow>(&translated);
    for id in ids {
        query = query.bind(id);
    }
    if let Some(tid) = tenant_id {
        query = query.bind(tid);
    }
    let rows = query.fetch_all(pool).await?;
    Ok(rows)
}

/// 根据 ID 列表统计已发布文章数量
///
/// 用于搜索引擎返回总数时进行验证，或作为后备计数。
pub async fn count_published_by_ids(
    pool: &crate::db::Pool,
    ids: &[String],
    tenant_id: Option<&str>,
) -> AppResult<i64> {
    if ids.is_empty() {
        return Ok(0);
    }

    let placeholders: Vec<&str> = ids.iter().map(|_| "?").collect();
    let sql = format!(
        "SELECT COUNT(*) FROM posts WHERE id IN ({}) AND status = 'published'{}",
        placeholders.join(","),
        tenant_filter(tenant_id)
    );
    let translated = crate::db::dialect::translate(&sql);
    let mut query = sqlx::query_as::<_, (i64,)>(&translated);
    for id in ids {
        query = query.bind(id);
    }
    if let Some(tid) = tenant_id {
        query = query.bind(tid);
    }
    let (count,) = query.fetch_one(pool).await?;
    Ok(count)
}
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
    tenant_id: Option<&str>,
) -> AppResult<(Vec<PostJoinedRow>, i64)> {
    let offset = (page - 1) * page_size;

    let (posts, total) = if let Some(tag_id) = tag_id {
        let filter = tenant_filter_aliased("p", tenant_id);
        let sql = format!(
            "{JOIN_SQL} \
             INNER JOIN posts_tags pt ON p.id = pt.post_id \
             WHERE p.status = 'published' AND pt.tag_id = ?{filter} \
             ORDER BY p.is_pinned DESC, p.created_at DESC LIMIT ? OFFSET ?"
        );
        let sql = crate::db::dialect::translate(&sql);
        let mut query = sqlx::query_as::<_, PostJoinedRow>(&sql).bind(tag_id);
        if let Some(tid) = tenant_id {
            query = query.bind(tid);
        }
        let posts = query.bind(page_size).bind(offset).fetch_all(pool).await?;

        let sql = format!(
            "SELECT COUNT(*) FROM posts p INNER JOIN posts_tags pt ON p.id = pt.post_id WHERE p.status = 'published' AND pt.tag_id = ?{filter}"
        );
        let sql = crate::db::dialect::translate(&sql);
        let mut query = sqlx::query_as::<_, (i64,)>(&sql).bind(tag_id);
        if let Some(tid) = tenant_id {
            query = query.bind(tid);
        }
        let total = query.fetch_one(pool).await?;

        (posts, total.0)
    } else if let Some(q) = q {
        let pattern = format!("%{q}%");
        let filter = tenant_filter_aliased("p", tenant_id);
        let sql = format!(
            "{JOIN_SQL} \
             WHERE p.status = 'published'{filter} AND (p.title LIKE ? OR p.content LIKE ?) \
             ORDER BY p.is_pinned DESC, p.created_at DESC LIMIT ? OFFSET ?"
        );
        let sql = crate::db::dialect::translate(&sql);
        let mut query = sqlx::query_as::<_, PostJoinedRow>(&sql);
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

        let filter = tenant_filter(tenant_id);
        let sql = format!(
            "SELECT COUNT(*) FROM posts WHERE status = 'published'{filter} AND (title LIKE ? OR content LIKE ?)"
        );
        let sql = crate::db::dialect::translate(&sql);
        let mut query = sqlx::query_as::<_, (i64,)>(&sql);
        if let Some(tid) = tenant_id {
            query = query.bind(tid);
        }
        let total = query.bind(&pattern).bind(&pattern).fetch_one(pool).await?;

        (posts, total.0)
    } else if let Some(category_id) = category_id {
        let filter = tenant_filter_aliased("p", tenant_id);
        let sql = format!(
            "{JOIN_SQL} \
             WHERE p.status = 'published' AND p.category_id = ?{filter} \
             ORDER BY p.is_pinned DESC, p.created_at DESC LIMIT ? OFFSET ?"
        );
        let sql = crate::db::dialect::translate(&sql);
        let mut query = sqlx::query_as::<_, PostJoinedRow>(&sql).bind(category_id);
        if let Some(tid) = tenant_id {
            query = query.bind(tid);
        }
        let posts = query.bind(page_size).bind(offset).fetch_all(pool).await?;

        let filter = tenant_filter(tenant_id);
        let sql = format!(
            "SELECT COUNT(*) FROM posts WHERE status = 'published' AND category_id = ?{filter}"
        );
        let sql = crate::db::dialect::translate(&sql);
        let mut query = sqlx::query_as::<_, (i64,)>(&sql).bind(category_id);
        if let Some(tid) = tenant_id {
            query = query.bind(tid);
        }
        let total = query.fetch_one(pool).await?;

        (posts, total.0)
    } else {
        let filter = tenant_filter_aliased("p", tenant_id);
        let sql = format!(
            "{JOIN_SQL} \
             WHERE p.status = 'published'{filter} \
             ORDER BY p.is_pinned DESC, p.created_at DESC LIMIT ? OFFSET ?"
        );
        let sql = crate::db::dialect::translate(&sql);
        let mut query = sqlx::query_as::<_, PostJoinedRow>(&sql);
        if let Some(tid) = tenant_id {
            query = query.bind(tid);
        }
        let posts = query.bind(page_size).bind(offset).fetch_all(pool).await?;

        let filter = tenant_filter(tenant_id);
        let sql = format!("SELECT COUNT(*) FROM posts WHERE status = 'published'{filter}");
        let sql = crate::db::dialect::translate(&sql);
        let mut query = sqlx::query_as::<_, (i64,)>(&sql);
        if let Some(tid) = tenant_id {
            query = query.bind(tid);
        }
        let total = query.fetch_one(pool).await?;

        (posts, total.0)
    };

    Ok((posts, total))
}
