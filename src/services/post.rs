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

use crate::commands::{
    CreateCategoryCmd, CreatePostCmd, FindPublishedQuery, UpdateCategoryCmd, UpdatePostCmd,
};
use crate::errors::app_error::{AppError, AppResult};
use crate::eventbus::{Event, EventBus};
use crate::handlers::dto::CreateTagRequest;
use crate::handlers::dto::{CreateCategoryRequest, UpdateCategoryRequest};
use crate::handlers::dto::{CreatePostRequest, PostResponse, UpdatePostRequest};
use crate::models::post::PostJoinedRow;
use crate::plugins::{HookPoint, PluginManager};
use crate::repositories::{CategoryRepository, PostRepository, TagRepository};
use crate::search::SearchEngine;
use crate::utils::markdown::render_markdown;

async fn render_content(content: &str, plugins: &PluginManager) -> String {
    match plugins.dispatch_render_override(content).await {
        Some(html) => plugins
            .dispatch_filter(HookPoint::FilterHtml, html)
            .await
            .unwrap_or_else(|e| {
                tracing::warn!("filter_html hook failed: {e}");
                render_markdown(content)
            }),
        None => render_markdown(content),
    }
}

async fn joined_row_to_response(
    r: PostJoinedRow,
    tags: Vec<crate::models::post::TagBrief>,
    plugins: &PluginManager,
) -> PostResponse {
    let html_content = render_content(&r.content, plugins).await;
    PostResponse {
        id: r.id,
        title: r.title,
        slug: r.slug,
        content: r.content,
        html_content,
        excerpt: r.excerpt,
        cover_image: r.cover_image,
        status: r.status,
        author_id: r.author_id,
        author_name: r.author_name,
        category_id: r.category_id,
        category_name: r.category_name,
        tags,
        view_count: r.view_count,
        is_pinned: r.is_pinned,
        created_at: r.created_at,
        updated_at: r.updated_at,
        published_at: r.published_at,
        title_highlight: None,
        excerpt_highlight: None,
    }
}

async fn build_post_response_from_repo(
    repo: &dyn PostRepository,
    id: &str,
    plugins: &PluginManager,
    tenant_id: Option<&str>,
) -> AppResult<PostResponse> {
    let row = repo.find_joined_by_id(id, tenant_id).await?;
    let tags = repo
        .get_post_tags(&row.id, tenant_id)
        .await
        .unwrap_or_default();
    Ok(joined_row_to_response(row, tags, plugins).await)
}

/// 创建分类。
///
/// 从分类名称自动生成 slug。
pub async fn create_category(
    category_repo: &dyn CategoryRepository,
    req: CreateCategoryRequest,
    tenant_id: Option<&str>,
) -> AppResult<crate::models::category::Category> {
    let slug = slugify(&req.name);
    category_repo
        .create(
            CreateCategoryCmd {
                name: req.name,
                slug,
                description: req.description,
                parent_id: req.parent_id,
                sort_order: req.sort_order.unwrap_or(0),
            },
            tenant_id,
        )
        .await
}

/// 更新分类。
///
/// 若名称变更，自动重新生成 slug。
pub async fn update_category(
    category_repo: &dyn CategoryRepository,
    id: &str,
    req: UpdateCategoryRequest,
    tenant_id: Option<&str>,
) -> AppResult<crate::models::category::Category> {
    let existing = category_repo.find_by_id(id, tenant_id).await?;
    let new_slug = req.name.as_ref().map(slugify).unwrap_or(existing.slug);

    category_repo
        .update(
            UpdateCategoryCmd {
                id: id.to_string(),
                name: req.name,
                slug: Some(new_slug),
                description: req.description,
                parent_id: req.parent_id,
                sort_order: req.sort_order,
            },
            tenant_id,
        )
        .await
}

/// 删除分类。
pub async fn delete_category(
    category_repo: &dyn CategoryRepository,
    id: &str,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    category_repo.delete(id, tenant_id).await
}

/// 获取所有分类列表。
pub async fn list_categories(
    category_repo: &dyn CategoryRepository,
    tenant_id: Option<&str>,
) -> AppResult<Vec<crate::models::category::Category>> {
    category_repo.find_all(tenant_id).await
}

/// 创建标签。
///
/// 从标签名称自动生成 slug。
pub async fn create_tag(
    tag_repo: &dyn TagRepository,
    req: CreateTagRequest,
    tenant_id: Option<&str>,
) -> AppResult<crate::models::tag::Tag> {
    let slug = slugify(&req.name);
    tag_repo.create(&req.name, &slug, tenant_id).await
}

/// 删除标签。
pub async fn delete_tag(
    tag_repo: &dyn TagRepository,
    id: &str,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    tag_repo.delete(id, tenant_id).await
}

/// 获取所有标签列表。
pub async fn list_tags(
    tag_repo: &dyn TagRepository,
    tenant_id: Option<&str>,
) -> AppResult<Vec<crate::models::tag::Tag>> {
    tag_repo.find_all(tenant_id).await
}

/// 生成唯一的 slug。
///
/// 若基础 slug 已被占用，则追加递增后缀（`-2`、`-3`、...）直到唯一。
async fn make_unique_slug(
    base_slug: &str,
    repo: &dyn PostRepository,
    tenant_id: Option<&str>,
) -> AppResult<String> {
    let mut slug = base_slug.to_string();
    let mut counter = 1;
    while repo.find_by_slug(&slug, tenant_id).await?.is_some() {
        slug = format!("{base_slug}-{counter}");
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

/// 创建文章。
///
/// - 从标题自动生成唯一 slug。
/// - 若未提供摘要，从内容中自动提取前 200 字符。
/// - 通过 Repository 创建文章并同步关联标签，确保原子性。
pub async fn create_post(
    repo: &dyn PostRepository,
    plugins: &PluginManager,
    eventbus: &EventBus,
    author_id: &str,
    req: CreatePostRequest,
    tenant_id: Option<&str>,
) -> AppResult<PostResponse> {
    let req = plugins
        .dispatch_filter(HookPoint::PostCreating, req)
        .await?;
    let base_slug = slugify(&req.title);
    let slug = make_unique_slug(&base_slug, repo, tenant_id).await?;
    let status = req.status.as_deref().unwrap_or("draft");
    let excerpt = req.excerpt.as_deref().map_or_else(
        || extract_excerpt(&req.content, 200),
        std::string::ToString::to_string,
    );

    let p = repo
        .create(
            CreatePostCmd {
                title: req.title,
                slug,
                content: req.content,
                excerpt: Some(excerpt),
                cover_image: req.cover_image,
                status: status.to_string(),
                author_id: author_id.to_string(),
                category_id: req.category_id.filter(|s| !s.is_empty()),
                tag_ids: req.tag_ids,
            },
            tenant_id,
        )
        .await?;

    let resp = build_post_response_from_repo(repo, &p.id, plugins, tenant_id).await?;
    eventbus.emit(Event::PostCreated {
        id: p.id.clone(),
        slug: resp.slug.clone(),
        title: resp.title.clone(),
        author_id: author_id.to_string(),
    });
    Ok(resp)
}

/// 更新文章。
///
/// - 若标题变更，重新生成唯一 slug。
/// - 重新生成摘要（若内容变更）。
/// - 通过 Repository 更新文章并同步关联标签，确保原子性。
pub async fn update_post(
    repo: &dyn PostRepository,
    plugins: &PluginManager,
    id: &str,
    req: UpdatePostRequest,
    tenant_id: Option<&str>,
) -> AppResult<PostResponse> {
    let req = plugins
        .dispatch_filter(HookPoint::PostUpdating, req)
        .await?;

    let existing = repo
        .find_by_id(id, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found("post"))?;

    let new_slug = req
        .title
        .as_ref()
        .map(slugify)
        .filter(|s| s != &existing.slug);

    let slug = match new_slug {
        Some(s) => Some(make_unique_slug(&s, repo, tenant_id).await?),
        None => None,
    };

    let content = req.content.as_deref().unwrap_or(&existing.content);
    let excerpt = req
        .excerpt
        .clone()
        .unwrap_or_else(|| extract_excerpt(content, 200));

    repo.update(
        UpdatePostCmd {
            id: id.to_string(),
            title: req.title,
            slug,
            content: Some(content.to_string()),
            excerpt: Some(excerpt),
            cover_image: req.cover_image,
            status: req.status,
            category_id: req.category_id.filter(|s| !s.is_empty()),
            tag_ids: req.tag_ids,
        },
        tenant_id,
    )
    .await?;

    build_post_response_from_repo(repo, id, plugins, tenant_id).await
}

/// 删除文章。
pub async fn delete_post(
    repo: &dyn PostRepository,
    id: &str,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    repo.delete(id, tenant_id).await
}

/// 带权限校验的文章更新。
///
/// 仅文章作者或管理员可执行。
#[allow(clippy::too_many_arguments)]
pub async fn update_post_with_auth(
    repo: &dyn PostRepository,
    plugins: &PluginManager,
    eventbus: &EventBus,
    slug: &str,
    user_id: &str,
    role: &str,
    req: UpdatePostRequest,
    tenant_id: Option<&str>,
) -> AppResult<PostResponse> {
    let existing = repo
        .find_by_slug(slug, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found("post"))?;

    crate::utils::auth::require_owner_or_admin(role, user_id, &existing.author_id)?;

    let resp = update_post(repo, plugins, &existing.id, req, tenant_id).await?;
    eventbus.emit(Event::PostUpdated {
        id: existing.id.clone(),
        slug: resp.slug.clone(),
    });
    Ok(resp)
}

/// 带权限校验的文章删除。
///
/// 仅文章作者或管理员可执行。
#[allow(clippy::too_many_arguments)]
pub async fn delete_post_with_auth(
    repo: &dyn PostRepository,
    _plugins: &PluginManager,
    eventbus: &EventBus,
    slug: &str,
    user_id: &str,
    role: &str,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    let existing = repo
        .find_by_slug(slug, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found("post"))?;

    crate::utils::auth::require_owner_or_admin(role, user_id, &existing.author_id)?;

    let id = existing.id.clone();
    let slug = slug.to_string();
    delete_post(repo, &existing.id, tenant_id).await?;
    eventbus.emit(Event::PostDeleted { id, slug });
    Ok(())
}

/// 获取已发布文章的详情。
///
/// 每次访问原子递增文章的浏览计数（`view_count`），
/// 并通过 JOIN 一次查询获取作者名和分类名。
pub async fn get_post(
    repo: &dyn PostRepository,
    slug: &str,
    plugins: &PluginManager,
    tenant_id: Option<&str>,
) -> AppResult<PostResponse> {
    let row = repo.increment_view_count_joined(slug, tenant_id).await?;
    let tags = repo
        .get_post_tags(&row.id, tenant_id)
        .await
        .unwrap_or_default();
    Ok(joined_row_to_response(row, tags, plugins).await)
}

/// 分页查询已发布文章列表。
///
/// 支持按分类、标签和关键词进行可选过滤。
/// 当提供 `search` 且有关键词时，优先使用搜索引擎（Tantivy）进行查询；
/// 若搜索引擎不可用或查询为空，则回退到 SQL LIKE 查询。
/// 使用 JOIN 查询和批量标签获取，将查询次数从 3N+1 降至 2~3 次。
#[allow(clippy::too_many_arguments)]
pub async fn list_posts(
    repo: &dyn PostRepository,
    page: i64,
    page_size: i64,
    category_id: Option<&str>,
    tag_id: Option<&str>,
    q: Option<&str>,
    plugins: &PluginManager,
    search: Option<&dyn SearchEngine>,
    tenant_id: Option<&str>,
) -> AppResult<(Vec<PostResponse>, i64)> {
    let (rows, total, highlights) = if let (Some(engine), Some(keyword)) = (search, q) {
        if !engine.is_noop() && !keyword.is_empty() {
            let (results, total) = engine.search(keyword, page, page_size).await?;
            let mut hmap = std::collections::HashMap::new();
            let ids: Vec<String> = results
                .into_iter()
                .map(|r| {
                    hmap.insert(r.post_id.clone(), (r.title_highlight, r.excerpt_highlight));
                    r.post_id
                })
                .collect();
            let rows = repo.find_joined_by_ids(&ids, tenant_id).await?;
            (rows, total, hmap)
        } else {
            let (rows, total) = repo
                .find_published_joined(
                    FindPublishedQuery {
                        page,
                        page_size,
                        category_id: category_id.map(std::string::ToString::to_string),
                        tag_id: tag_id.map(std::string::ToString::to_string),
                        q: if keyword.is_empty() {
                            None
                        } else {
                            Some(keyword.to_string())
                        },
                    },
                    tenant_id,
                )
                .await?;
            (rows, total, std::collections::HashMap::new())
        }
    } else {
        let (rows, total) = repo
            .find_published_joined(
                FindPublishedQuery {
                    page,
                    page_size,
                    category_id: category_id.map(std::string::ToString::to_string),
                    tag_id: tag_id.map(std::string::ToString::to_string),
                    q: q.map(std::string::ToString::to_string),
                },
                tenant_id,
            )
            .await?;
        (rows, total, std::collections::HashMap::new())
    };

    let post_ids: Vec<String> = rows.iter().map(|r| r.id.clone()).collect();
    let tags_map = repo
        .get_tags_for_posts(&post_ids, tenant_id)
        .await
        .unwrap_or_default();

    let mut responses = Vec::with_capacity(rows.len());
    for r in rows {
        let html_content = render_content(&r.content, plugins).await;
        let (title_hl, excerpt_hl) = highlights
            .get(&r.id)
            .map_or((None, None), |(t, e)| (t.clone(), e.clone()));
        responses.push(PostResponse {
            id: r.id.clone(),
            title: r.title,
            slug: r.slug,
            content: r.content,
            html_content,
            excerpt: r.excerpt,
            cover_image: r.cover_image,
            status: r.status,
            author_id: r.author_id,
            author_name: r.author_name,
            category_id: r.category_id,
            category_name: r.category_name,
            tags: tags_map.get(&r.id).cloned().unwrap_or_default(),
            view_count: r.view_count,
            is_pinned: r.is_pinned,
            created_at: r.created_at,
            updated_at: r.updated_at,
            published_at: r.published_at,
            title_highlight: title_hl,
            excerpt_highlight: excerpt_hl,
        });
    }

    Ok((responses, total))
}

/// 后台管理：按 slug 获取文章详情（不过滤状态，不增加浏览量）
pub async fn get_post_any_status(
    repo: &dyn PostRepository,
    slug: &str,
    plugins: &PluginManager,
    tenant_id: Option<&str>,
) -> AppResult<PostResponse> {
    let post = repo.find_by_slug(slug, tenant_id).await?;
    let post =
        post.ok_or_else(|| crate::errors::app_error::AppError::not_found("post not found"))?;
    let row = repo.find_joined_by_id(&post.id, tenant_id).await?;
    let tags = repo
        .get_post_tags(&row.id, tenant_id)
        .await
        .unwrap_or_default();
    Ok(joined_row_to_response(row, tags, plugins).await)
}

/// 后台管理：分页查询全部文章（含所有状态）
pub async fn list_all_posts(
    repo: &dyn PostRepository,
    page: i64,
    page_size: i64,
    status: Option<&str>,
    plugins: &PluginManager,
    tenant_id: Option<&str>,
) -> AppResult<(Vec<PostResponse>, i64)> {
    let (rows, total) = repo
        .find_all_joined(page, page_size, status, tenant_id)
        .await?;

    let post_ids: Vec<String> = rows.iter().map(|r| r.id.clone()).collect();
    let tags_map = repo
        .get_tags_for_posts(&post_ids, tenant_id)
        .await
        .unwrap_or_default();

    let mut responses = Vec::with_capacity(rows.len());
    for r in rows {
        let html_content = render_content(&r.content, plugins).await;
        responses.push(PostResponse {
            id: r.id.clone(),
            title: r.title,
            slug: r.slug,
            content: r.content,
            html_content,
            excerpt: r.excerpt,
            cover_image: r.cover_image,
            status: r.status,
            author_id: r.author_id,
            author_name: r.author_name,
            category_id: r.category_id,
            category_name: r.category_name,
            tags: tags_map.get(&r.id).cloned().unwrap_or_default(),
            view_count: r.view_count,
            is_pinned: r.is_pinned,
            created_at: r.created_at,
            updated_at: r.updated_at,
            published_at: r.published_at,
            title_highlight: None,
            excerpt_highlight: None,
        });
    }

    Ok((responses, total))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_excerpt_short_content() {
        let content = "short";
        let result = extract_excerpt(content, 200);
        assert_eq!(result, "short");
    }

    #[test]
    fn extract_excerpt_truncates_long_content() {
        let content = "a".repeat(300);
        let result = extract_excerpt(&content, 200);
        assert!(result.ends_with("..."));
        assert_eq!(result.len(), 203);
    }

    #[test]
    fn extract_excerpt_exact_boundary() {
        let content = "a".repeat(200);
        let result = extract_excerpt(&content, 200);
        assert_eq!(result, "a".repeat(200));
    }

    #[test]
    fn extract_excerpt_unicode_safe() {
        let content = "你好世界".repeat(100);
        let result = extract_excerpt(&content, 200);
        assert!(result.ends_with("...") || result.len() <= 200);
    }
}
