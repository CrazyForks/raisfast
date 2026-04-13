//! 文章、分类和标签服务。
//!
//! 提供文章（含分类、标签）的完整 CRUD 业务逻辑，包括：
//!
//! - 分类和标签的创建、更新、删除、列表查询
//! - 文章的创建、更新、删除、发布态查询和详情查询
//! - Slug 自动生成与去重
//! - 内容摘要自动提取
//! - 文章响应对象的组装（含 HTML 渲染、标签和作者信息）

use slug::slugify;

use crate::errors::app_error::{AppError, AppResult};
use crate::models::category::{self, CreateCategoryRequest, UpdateCategoryRequest};
use crate::models::post::{self, CreatePostRequest, PostResponse, UpdatePostRequest};
use crate::models::tag::{self, CreateTagRequest};
use crate::utils::markdown::render_markdown;

/// 创建分类。
///
/// 从分类名称自动生成 slug。
pub async fn create_category(
    pool: &sqlx::SqlitePool,
    req: CreateCategoryRequest,
) -> AppResult<category::Category> {
    let slug = slugify(&req.name);
    category::create(
        pool,
        &req.name,
        &slug,
        req.description.as_deref(),
        req.parent_id.as_deref(),
        req.sort_order.unwrap_or(0),
    )
    .await
}

/// 更新分类。
///
/// 若名称变更，自动重新生成 slug。
pub async fn update_category(
    pool: &sqlx::SqlitePool,
    id: &str,
    req: UpdateCategoryRequest,
) -> AppResult<category::Category> {
    let existing = category::find_by_id(pool, id).await?;
    let new_slug = req.name.as_ref().map(slugify).unwrap_or(existing.slug);

    category::update(
        pool,
        id,
        req.name.as_deref(),
        Some(&new_slug),
        req.description.as_deref(),
        req.parent_id.as_deref(),
        req.sort_order,
    )
    .await
}

/// 删除分类。
pub async fn delete_category(pool: &sqlx::SqlitePool, id: &str) -> AppResult<()> {
    category::delete(pool, id).await
}

/// 获取所有分类列表。
pub async fn list_categories(pool: &sqlx::SqlitePool) -> AppResult<Vec<category::Category>> {
    category::find_all(pool).await
}

/// 创建标签。
///
/// 从标签名称自动生成 slug。
pub async fn create_tag(pool: &sqlx::SqlitePool, req: CreateTagRequest) -> AppResult<tag::Tag> {
    let slug = slugify(&req.name);
    tag::create(pool, &req.name, &slug).await
}

/// 删除标签。
pub async fn delete_tag(pool: &sqlx::SqlitePool, id: &str) -> AppResult<()> {
    tag::delete(pool, id).await
}

/// 获取所有标签列表。
pub async fn list_tags(pool: &sqlx::SqlitePool) -> AppResult<Vec<tag::Tag>> {
    tag::find_all(pool).await
}

/// 生成唯一的 slug。
///
/// 若基础 slug 已被占用，则追加递增后缀（`-2`、`-3`、...）直到唯一。
async fn make_unique_slug(base_slug: &str, pool: &sqlx::SqlitePool) -> AppResult<String> {
    let mut slug = base_slug.to_string();
    let mut counter = 1;
    while post::find_by_slug(pool, &slug).await?.is_some() {
        slug = format!("{}-{}", base_slug, counter);
        counter += 1;
    }
    Ok(slug)
}

/// 从文章内容中提取摘要。
///
/// 取前 `max_len` 个字符作为摘要，超出部分以 `"..."` 结尾。
fn extract_excerpt(content: &str, max_len: usize) -> String {
    let plain = content.chars().take(max_len * 2).collect::<String>();
    if plain.len() > max_len {
        format!("{}...", &plain[..plain.ceil_char_boundary(max_len)])
    } else {
        plain
    }
}

/// 组装文章响应对象。
///
/// 将原始文章记录转换为完整的 [`PostResponse`]，包括：
/// - Markdown 内容渲染为 HTML
/// - 关联的标签列表
/// - 作者名称和分类名称
async fn build_post_response(pool: &sqlx::SqlitePool, p: post::Post) -> AppResult<PostResponse> {
    let html_content = render_markdown(&p.content);
    let tags = post::get_post_tags(pool, &p.id).await.unwrap_or_default();
    let author_name = post::get_author_name(pool, &p.author_id)
        .await
        .unwrap_or(None);
    let category_name = match &p.category_id {
        Some(cid) => post::get_category_name(pool, cid).await.unwrap_or(None),
        None => None,
    };

    Ok(PostResponse {
        id: p.id,
        title: p.title,
        slug: p.slug,
        content: p.content,
        html_content,
        excerpt: p.excerpt,
        cover_image: p.cover_image,
        status: p.status,
        author_id: p.author_id,
        author_name,
        category_id: p.category_id,
        category_name,
        tags,
        view_count: p.view_count,
        is_pinned: p.is_pinned,
        created_at: p.created_at,
        updated_at: p.updated_at,
        published_at: p.published_at,
    })
}

/// 创建文章。
///
/// - 从标题自动生成唯一 slug。
/// - 若未提供摘要，从内容中自动提取前 200 字符。
/// - 同步关联标签。
pub async fn create_post(
    pool: &sqlx::SqlitePool,
    author_id: &str,
    req: CreatePostRequest,
) -> AppResult<PostResponse> {
    let base_slug = slugify(&req.title);
    let slug = make_unique_slug(&base_slug, pool).await?;
    let status = req.status.as_deref().unwrap_or("draft");
    let excerpt = req
        .excerpt
        .as_deref()
        .map(|s| s.to_string())
        .unwrap_or_else(|| extract_excerpt(&req.content, 200));

    let p = post::create(
        pool,
        &req.title,
        &slug,
        &req.content,
        Some(&excerpt),
        req.cover_image.as_deref(),
        status,
        author_id,
        req.category_id.as_deref(),
    )
    .await?;

    if let Some(tag_ids) = &req.tag_ids {
        post::sync_tags(pool, &p.id, tag_ids).await?;
    }

    build_post_response(pool, p).await
}

/// 更新文章。
///
/// - 若标题变更，重新生成唯一 slug。
/// - 重新生成摘要（若内容变更）。
/// - 同步关联标签。
pub async fn update_post(
    pool: &sqlx::SqlitePool,
    id: &str,
    req: UpdatePostRequest,
) -> AppResult<PostResponse> {
    let existing = post::find_by_id(pool, id)
        .await?
        .ok_or_else(|| AppError::NotFound("post".into()))?;

    let new_slug = req
        .title
        .as_ref()
        .map(slugify)
        .filter(|s| s != &existing.slug);

    let slug = match new_slug {
        Some(s) => Some(make_unique_slug(&s, pool).await?),
        None => None,
    };

    let content = req.content.as_deref().unwrap_or(&existing.content);
    let excerpt = req
        .excerpt
        .clone()
        .unwrap_or_else(|| extract_excerpt(content, 200));

    let p = post::update(
        pool,
        id,
        req.title.as_deref(),
        slug.as_deref(),
        Some(content),
        Some(&excerpt),
        req.cover_image.as_deref(),
        req.status.as_deref(),
        req.category_id.as_deref(),
    )
    .await?;

    if let Some(tag_ids) = &req.tag_ids {
        post::sync_tags(pool, &p.id, tag_ids).await?;
    }

    build_post_response(pool, p).await
}

/// 删除文章。
pub async fn delete_post(pool: &sqlx::SqlitePool, id: &str) -> AppResult<()> {
    post::delete(pool, id).await
}

/// 带权限校验的文章更新。
///
/// 仅文章作者或管理员可执行。
pub async fn update_post_with_auth(
    pool: &sqlx::SqlitePool,
    slug: &str,
    user_id: &str,
    role: &str,
    req: UpdatePostRequest,
) -> AppResult<PostResponse> {
    let existing = post::find_by_slug(pool, slug)
        .await?
        .ok_or_else(|| AppError::NotFound("post".into()))?;

    if role != "admin" && existing.author_id != user_id {
        return Err(AppError::Forbidden);
    }

    update_post(pool, &existing.id, req).await
}

/// 带权限校验的文章删除。
///
/// 仅文章作者或管理员可执行。
pub async fn delete_post_with_auth(
    pool: &sqlx::SqlitePool,
    slug: &str,
    user_id: &str,
    role: &str,
) -> AppResult<()> {
    let existing = post::find_by_slug(pool, slug)
        .await?
        .ok_or_else(|| AppError::NotFound("post".into()))?;

    if role != "admin" && existing.author_id != user_id {
        return Err(AppError::Forbidden);
    }

    delete_post(pool, &existing.id).await
}

/// 获取已发布文章的详情。
///
/// 每次访问自动递增文章的浏览计数（`view_count`）。
pub async fn get_post(pool: &sqlx::SqlitePool, slug: &str) -> AppResult<PostResponse> {
    let p = post::find_published_by_slug(pool, slug).await?;
    let _ = post::increment_view_count(pool, &p.id).await;
    build_post_response(pool, p).await
}

/// 分页查询已发布文章列表。
///
/// 支持按分类、标签和关键词进行可选过滤。
/// 返回文章响应列表和总记录数。
pub async fn list_posts(
    pool: &sqlx::SqlitePool,
    page: i64,
    page_size: i64,
    category_id: Option<&str>,
    tag_id: Option<&str>,
    q: Option<&str>,
) -> AppResult<(Vec<PostResponse>, i64)> {
    let (posts, total) =
        post::find_published(pool, page, page_size, category_id, tag_id, q).await?;

    let mut responses = Vec::with_capacity(posts.len());
    for p in posts {
        responses.push(build_post_response(pool, p).await?);
    }

    Ok((responses, total))
}

/// 获取文章详情（供所有者编辑用）。
///
/// 与 [`get_post`] 不同，此方法返回文章不论其发布状态，适用于作者编辑草稿。
pub async fn get_post_for_owner(pool: &sqlx::SqlitePool, id: &str) -> AppResult<PostResponse> {
    let p = post::find_by_id(pool, id)
        .await?
        .ok_or_else(|| AppError::NotFound("post".into()))?;
    build_post_response(pool, p).await
}
