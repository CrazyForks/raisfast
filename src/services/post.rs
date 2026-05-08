//! 文章服务。
//!
//! 提供文章的完整 CRUD 业务逻辑，包括：
//!
//! - 文章的创建、更新、删除、发布态查询和详情查询
//! - Slug 自动生成与去重
//! - 内容摘要自动提取
//! - 文章响应对象的组装（含标签和作者信息）

use slug::slugify;

use crate::commands::{CreatePostCmd, FindPublishedQuery, UpdatePostCmd};
use crate::dto::{CreatePostRequest, PostResponse, UpdatePostRequest};
use crate::errors::app_error::{AppError, AppResult};
use crate::eventbus::{Event, EventBus};
use crate::middleware::auth::AuthUser;
use crate::models::post::PostJoinedRow;
use crate::plugins::{HookPoint, PluginManager};
use crate::repositories::PostRepository;
use crate::search::SearchEngine;

pub async fn resolve_doc_id_to_int(
    pool: &crate::db::Pool,
    table: &str,
    doc_id: &str,
    tenant_id: Option<&str>,
) -> AppResult<Option<i64>> {
    if doc_id.is_empty() {
        return Ok(None);
    }
    if let Ok(int_id) = doc_id.parse::<i64>() {
        return Ok(Some(int_id));
    }
    if !crate::db::dialect::is_safe_identifier(table) {
        return Ok(None);
    }
    let filter = if tenant_id.is_some() {
        format!(" AND tenant_id = {}", crate::db::dialect::ph(2))
    } else {
        String::new()
    };
    let sql = format!(
        "SELECT id FROM {table} WHERE document_id = {}{filter}",
        crate::db::dialect::ph(1)
    );
    let mut q = sqlx::query_scalar::<_, i64>(&sql).bind(doc_id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    q.fetch_optional(pool)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("resolve doc_id in {table} failed: {e}")))
}

async fn joined_row_to_response(
    r: PostJoinedRow,
    tags: Vec<crate::models::post::TagBrief>,
    _plugins: &PluginManager,
) -> PostResponse {
    PostResponse {
        id: r.document_id,
        title: r.title,
        slug: r.slug,
        content: r.content,
        excerpt: r.excerpt,
        cover_image: r.cover_image,
        status: r.status,
        created_by: r.created_by,
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

async fn build_response_from_post(
    post: &crate::models::post::Post,
    author_name: Option<String>,
    category_name: Option<String>,
    tags: Vec<crate::models::post::TagBrief>,
) -> PostResponse {
    PostResponse {
        id: post.document_id.clone(),
        title: post.title.clone(),
        slug: post.slug.clone(),
        content: post.content.clone(),
        excerpt: post.excerpt.clone(),
        cover_image: post.cover_image.clone(),
        status: post.status.clone(),
        created_by: post.created_by,
        author_name,
        category_id: post.category_id,
        category_name,
        tags,
        view_count: post.view_count,
        is_pinned: post.is_pinned,
        created_at: post.created_at.clone(),
        updated_at: post.updated_at.clone(),
        published_at: post.published_at.clone(),
        title_highlight: None,
        excerpt_highlight: None,
    }
}

async fn build_post_response_from_repo(
    repo: &dyn PostRepository,
    id: i64,
    plugins: &PluginManager,
    auth: &AuthUser,
) -> AppResult<PostResponse> {
    let row = repo.find_joined_by_id(id, auth.tenant_id()).await?;
    let tags = repo
        .get_post_tags(row.id, auth.tenant_id())
        .await
        .unwrap_or_default();
    Ok(joined_row_to_response(row, tags, plugins).await)
}

fn make_unique_slug(base_slug: &str) -> String {
    let suffix = crate::utils::id::random_hex(2);
    format!("{base_slug}-{suffix}")
}

fn extract_excerpt(content: &str, max_len: usize) -> String {
    let plain = content
        .chars()
        .take(max_len.saturating_mul(2))
        .collect::<String>();
    if plain.len() > max_len {
        format!("{}...", &plain[..plain.ceil_char_boundary(max_len)])
    } else {
        plain
    }
}

#[tracing::instrument(skip(repo, plugins, eventbus), fields(slug = tracing::field::Empty))]
#[allow(clippy::too_many_arguments)]
pub async fn create_post(
    repo: &dyn PostRepository,
    plugins: &PluginManager,
    eventbus: &EventBus,
    auth: &AuthUser,
    req: CreatePostRequest,
) -> AppResult<PostResponse> {
    let user_id = auth.user_id().unwrap_or_default();

    let req = plugins
        .dispatch_filter(HookPoint::PostCreating, req)
        .await?;
    let base_slug = slugify(&req.title);
    let slug = make_unique_slug(&base_slug);
    let status = req.status.as_deref().unwrap_or("draft");
    let excerpt = req.excerpt.as_deref().map_or_else(
        || extract_excerpt(&req.content, 200),
        std::string::ToString::to_string,
    );

    let category_id = if let Some(ref doc_id) = req.category_id {
        resolve_doc_id_to_int(repo.pool(), "categories", doc_id, auth.tenant_id()).await?
    } else {
        None
    };
    let tag_ids = match req.tag_ids {
        Some(ref ids) => {
            let mut resolved = Vec::new();
            for doc_id in ids {
                if let Some(int_id) =
                    resolve_doc_id_to_int(repo.pool(), "tags", doc_id, auth.tenant_id()).await?
                {
                    resolved.push(int_id);
                }
            }
            Some(resolved)
        }
        None => None,
    };

    let cmd = CreatePostCmd {
        title: req.title,
        slug,
        content: req.content,
        excerpt: Some(excerpt),
        cover_image: req.cover_image,
        status: status.to_string(),
        created_by: auth.user_int_id().ok_or(AppError::Unauthorized)?,
        updated_by: auth.user_int_id(),
        category_id,
        tag_ids,
    };
    let p = repo.create(cmd, auth.tenant_id()).await?;

    let author_name =
        crate::models::post::get_author_name(repo.pool(), p.created_by, auth.tenant_id())
            .await
            .ok()
            .flatten();

    let category_name = if let Some(cat_id) = p.category_id {
        crate::models::post::get_category_name(repo.pool(), cat_id, auth.tenant_id())
            .await
            .ok()
            .flatten()
    } else {
        None
    };

    let tags = repo
        .get_post_tags(p.id, auth.tenant_id())
        .await
        .unwrap_or_default();

    let resp = build_response_from_post(&p, author_name, category_name, tags).await;
    tracing::Span::current().record("slug", &resp.slug);
    eventbus.emit(Event::PostCreated {
        id: p.document_id.clone(),
        slug: resp.slug.clone(),
        title: resp.title.clone(),
        author_id: user_id.to_string(),
    });
    Ok(resp)
}

async fn update_post_inner(
    repo: &dyn PostRepository,
    plugins: &PluginManager,
    id: i64,
    req: UpdatePostRequest,
    auth: &AuthUser,
) -> AppResult<PostResponse> {
    let req = plugins
        .dispatch_filter(HookPoint::PostUpdating, req)
        .await?;

    let existing = repo
        .find_by_id(id, auth.tenant_id())
        .await?
        .ok_or_else(|| AppError::not_found("post"))?;

    let new_slug = req
        .title
        .as_ref()
        .map(slugify)
        .filter(|s| s != &existing.slug);

    let slug: Option<String> = new_slug.as_deref().map(make_unique_slug);
    let content = req.content.as_deref().unwrap_or(&existing.content);
    let excerpt = req
        .excerpt
        .clone()
        .unwrap_or_else(|| extract_excerpt(content, 200));

    let category_id = if let Some(ref doc_id) = req.category_id {
        resolve_doc_id_to_int(repo.pool(), "categories", doc_id, auth.tenant_id()).await?
    } else {
        None
    };
    let tag_ids = match req.tag_ids {
        Some(ref ids) => {
            let mut resolved = Vec::new();
            for doc_id in ids {
                if let Some(int_id) =
                    resolve_doc_id_to_int(repo.pool(), "tags", doc_id, auth.tenant_id()).await?
                {
                    resolved.push(int_id);
                }
            }
            Some(resolved)
        }
        None => None,
    };

    let cmd = UpdatePostCmd {
        id: existing.id,
        title: req.title,
        slug,
        content: Some(content.to_string()),
        excerpt: Some(excerpt),
        cover_image: req.cover_image,
        status: req.status,
        category_id,
        tag_ids,
        updated_by: auth.user_int_id(),
    };
    repo.update(cmd, auth.tenant_id()).await?;

    build_post_response_from_repo(repo, id, plugins, auth).await
}

async fn delete_post_inner(repo: &dyn PostRepository, id: i64, _auth: &AuthUser) -> AppResult<()> {
    repo.delete(id, _auth.tenant_id()).await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn update_post(
    repo: &dyn PostRepository,
    plugins: &PluginManager,
    eventbus: &EventBus,
    slug: &str,
    auth: &AuthUser,
    req: UpdatePostRequest,
) -> AppResult<PostResponse> {
    let existing = repo
        .find_by_slug(slug, auth.tenant_id())
        .await?
        .ok_or_else(|| AppError::not_found("post"))?;

    crate::utils::auth::require_owner_or_admin(
        auth.role(),
        auth.user_int_id().ok_or(AppError::Unauthorized)?,
        existing.created_by,
    )?;

    let resp = update_post_inner(repo, plugins, existing.id, req, auth).await?;
    eventbus.emit(Event::PostUpdated {
        id: existing.document_id.clone(),
        slug: resp.slug.clone(),
    });
    Ok(resp)
}

#[allow(clippy::too_many_arguments)]
pub async fn delete_post(
    repo: &dyn PostRepository,
    _plugins: &PluginManager,
    eventbus: &EventBus,
    slug: &str,
    auth: &AuthUser,
) -> AppResult<()> {
    let existing = repo
        .find_by_slug(slug, auth.tenant_id())
        .await?
        .ok_or_else(|| AppError::not_found("post"))?;

    crate::utils::auth::require_owner_or_admin(
        auth.role(),
        auth.user_int_id().ok_or(AppError::Unauthorized)?,
        existing.created_by,
    )?;

    let doc_id = existing.document_id.clone();
    let slug_val = slug.to_string();
    delete_post_inner(repo, existing.id, auth).await?;
    eventbus.emit(Event::PostDeleted {
        id: doc_id,
        slug: slug_val,
    });
    Ok(())
}

pub async fn get_post(
    repo: &dyn PostRepository,
    slug: &str,
    plugins: &PluginManager,
    auth: &AuthUser,
) -> AppResult<PostResponse> {
    let row = repo
        .increment_view_count_joined(slug, auth.tenant_id())
        .await?;
    let tags = repo
        .get_post_tags(row.id, auth.tenant_id())
        .await
        .unwrap_or_default();
    Ok(joined_row_to_response(row, tags, plugins).await)
}

#[allow(clippy::too_many_arguments)]
pub async fn list_posts(
    repo: &dyn PostRepository,
    page: i64,
    page_size: i64,
    category_id: Option<i64>,
    tag_id: Option<i64>,
    q: Option<&str>,
    _plugins: &PluginManager,
    search: Option<&dyn SearchEngine>,
    auth: &AuthUser,
) -> AppResult<(Vec<PostResponse>, i64)> {
    let (rows, total, highlights): (Vec<_>, _, std::collections::HashMap<i64, _>) =
        if let (Some(engine), Some(keyword)) = (search, q) {
            if !engine.is_noop() && !keyword.is_empty() {
                let (results, total) = engine.search(keyword, page, page_size).await?;
                let mut hmap = std::collections::HashMap::new();
                let ids: Vec<i64> = results
                    .into_iter()
                    .filter_map(|r| {
                        let pid: i64 = r.post_id.parse().ok()?;
                        hmap.insert(pid, (r.title_highlight, r.excerpt_highlight));
                        Some(pid)
                    })
                    .collect();
                let rows = repo.find_joined_by_ids(&ids, auth.tenant_id()).await?;
                (rows, total, hmap)
            } else {
                let (rows, total) = repo
                    .find_published_joined(
                        FindPublishedQuery {
                            page,
                            page_size,
                            category_id,
                            tag_id,
                            q: if keyword.is_empty() {
                                None
                            } else {
                                Some(keyword.to_string())
                            },
                        },
                        auth.tenant_id(),
                    )
                    .await?;
                let hmap = if keyword.is_empty() {
                    std::collections::HashMap::new()
                } else {
                    rows.iter()
                        .map(|r| {
                            let title_hl = crate::search::highlight_text(keyword, &r.title);
                            let excerpt_hl = r
                                .excerpt
                                .as_ref()
                                .map(|e| crate::search::highlight_text(keyword, e))
                                .or_else(|| {
                                    crate::search::make_excerpt(&r.content, keyword, 200)
                                        .map(|e| crate::search::highlight_text(keyword, &e))
                                });
                            (r.id, (Some(title_hl), excerpt_hl))
                        })
                        .collect()
                };
                (rows, total, hmap)
            }
        } else {
            let (rows, total) = repo
                .find_published_joined(
                    FindPublishedQuery {
                        page,
                        page_size,
                        category_id,
                        tag_id,
                        q: q.map(std::string::ToString::to_string),
                    },
                    auth.tenant_id(),
                )
                .await?;
            let hmap = if let Some(kw) = q {
                if kw.is_empty() {
                    std::collections::HashMap::new()
                } else {
                    rows.iter()
                        .map(|r| {
                            let title_hl = crate::search::highlight_text(kw, &r.title);
                            let excerpt_hl = r
                                .excerpt
                                .as_ref()
                                .map(|e| crate::search::highlight_text(kw, e))
                                .or_else(|| {
                                    crate::search::make_excerpt(&r.content, kw, 200)
                                        .map(|e| crate::search::highlight_text(kw, &e))
                                });
                            (r.id, (Some(title_hl), excerpt_hl))
                        })
                        .collect()
                }
            } else {
                std::collections::HashMap::new()
            };
            (rows, total, hmap)
        };

    let post_ids: Vec<i64> = rows.iter().map(|r: &PostJoinedRow| r.id).collect();
    let tags_map = repo
        .get_tags_for_posts(&post_ids, auth.tenant_id())
        .await
        .unwrap_or_default();

    let mut responses = Vec::with_capacity(rows.len());
    for r in rows {
        let (title_hl, excerpt_hl): (Option<String>, Option<String>) = highlights
            .get(&r.id)
            .map_or((None, None), |(t, e)| (t.clone(), e.clone()));
        responses.push(PostResponse {
            id: r.document_id.clone(),
            title: r.title,
            slug: r.slug,
            content: r.content,
            excerpt: r.excerpt,
            cover_image: r.cover_image,
            status: r.status,
            created_by: r.created_by,
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

pub async fn get_post_any_status(
    repo: &dyn PostRepository,
    slug: &str,
    plugins: &PluginManager,
    auth: &AuthUser,
) -> AppResult<PostResponse> {
    let post = repo.find_by_slug(slug, auth.tenant_id()).await?;
    let post =
        post.ok_or_else(|| crate::errors::app_error::AppError::not_found("post not found"))?;
    let row = repo.find_joined_by_id(post.id, auth.tenant_id()).await?;
    let tags = repo
        .get_post_tags(row.id, auth.tenant_id())
        .await
        .unwrap_or_default();
    Ok(joined_row_to_response(row, tags, plugins).await)
}

pub async fn list_all_posts(
    repo: &dyn PostRepository,
    page: i64,
    page_size: i64,
    status: Option<&str>,
    _plugins: &PluginManager,
    auth: &AuthUser,
) -> AppResult<(Vec<PostResponse>, i64)> {
    let (rows, total) = repo
        .find_all_joined(page, page_size, status, auth.tenant_id())
        .await?;

    let post_ids: Vec<i64> = rows.iter().map(|r| r.id).collect();
    let tags_map = repo
        .get_tags_for_posts(&post_ids, auth.tenant_id())
        .await
        .unwrap_or_default();

    let mut responses = Vec::with_capacity(rows.len());
    for r in rows {
        responses.push(PostResponse {
            id: r.document_id.clone(),
            title: r.title,
            slug: r.slug,
            content: r.content,
            excerpt: r.excerpt,
            cover_image: r.cover_image,
            status: r.status,
            created_by: r.created_by,
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

    #[test]
    fn make_unique_slug_has_suffix() {
        let slug = make_unique_slug("my-post");
        assert!(slug.starts_with("my-post-"));
        let suffix = &slug["my-post-".len()..];
        assert_eq!(suffix.len(), 4);
        assert!(suffix.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn make_unique_slug_different_each_call() {
        let s1 = make_unique_slug("test");
        let s2 = make_unique_slug("test");
        assert_ne!(s1, s2);
    }

    #[test]
    fn extract_excerpt_zero_max_len() {
        let content = "hello world";
        let result = extract_excerpt(content, 0);
        assert!(result.is_empty() || result.ends_with("..."));
    }
}
