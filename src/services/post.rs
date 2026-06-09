//! Post service.
//!
//! Provides full CRUD business logic for posts, including:
//!
//! - Post creation, update, deletion, published listing, and detail retrieval
//! - Slug generation via SlugAspect
//! - Excerpt extraction via ExcerptAspect
//! - Post response assembly (with tags and author info)

use std::sync::Arc;

use async_trait::async_trait;
use slug::slugify;

use crate::aspects::engine::AspectEngine;
use crate::aspects::slug_aspect;
use crate::commands::{CreatePostCmd, UpdatePostCmd};
use crate::dto::{CreatePostRequest, PostResponse, UpdatePostRequest};
use crate::errors::app_error::{AppError, AppResult};
use crate::event::*;
use crate::middleware::auth::AuthUser;
use crate::models::post::{PostJoinedRow, PostStatus};
use crate::policy::Policy;
use crate::search::SearchEngine;
use crate::types::snowflake_id::SnowflakeId;

// ─── Trait ───

#[async_trait]
pub trait PostService: Send + Sync {
    async fn create(&self, auth: &AuthUser, req: CreatePostRequest) -> AppResult<PostResponse>;
    async fn update(
        &self,
        auth: &AuthUser,
        slug: &str,
        req: UpdatePostRequest,
    ) -> AppResult<PostResponse>;
    async fn delete(&self, auth: &AuthUser, slug: &str) -> AppResult<()>;
    async fn get(&self, auth: &AuthUser, slug: &str) -> AppResult<PostResponse>;
    async fn get_any_status(&self, auth: &AuthUser, slug: &str) -> AppResult<PostResponse>;
    async fn admin_get_by_id(&self, auth: &AuthUser, id: SnowflakeId) -> AppResult<PostResponse>;
    async fn list(
        &self,
        auth: &AuthUser,
        page: i64,
        page_size: i64,
        category_id: Option<i64>,
        tag_id: Option<i64>,
        q: Option<&str>,
    ) -> AppResult<(Vec<PostResponse>, i64)>;
    async fn list_all(
        &self,
        auth: &AuthUser,
        page: i64,
        page_size: i64,
        status: Option<PostStatus>,
        keyword: Option<&str>,
    ) -> AppResult<(Vec<PostResponse>, i64)>;
    async fn admin_update(
        &self,
        auth: &AuthUser,
        id: SnowflakeId,
        req: UpdatePostRequest,
    ) -> AppResult<PostResponse>;
    async fn admin_delete(&self, auth: &AuthUser, id: SnowflakeId) -> AppResult<()>;
    async fn batch(&self, auth: &AuthUser, action: &str, ids: &[String]) -> AppResult<usize>;
    async fn list_recent_published(
        &self,
        limit: i64,
        tenant_id: Option<&str>,
    ) -> AppResult<Vec<PostJoinedRow>>;
}

// ─── Implementation ───

pub struct PostServiceImpl {
    pool: Arc<crate::db::Pool>,
    aspect_engine: Arc<AspectEngine>,
    search: Arc<dyn SearchEngine>,
}

impl PostServiceImpl {
    pub fn new(
        pool: Arc<crate::db::Pool>,
        aspect_engine: Arc<AspectEngine>,
        search: Arc<dyn SearchEngine>,
    ) -> Self {
        Self {
            pool,
            aspect_engine,
            search,
        }
    }

    async fn before_create(
        &self,
        auth: &AuthUser,
        req: CreatePostRequest,
    ) -> AppResult<(CreatePostRequest, crate::aspects::Dispatched)> {
        self.aspect_engine.before_create("posts", auth, req).await
    }

    async fn before_update(
        &self,
        auth: &AuthUser,
        existing: &crate::models::post::Post,
        req: UpdatePostRequest,
    ) -> AppResult<(UpdatePostRequest, crate::aspects::Dispatched)> {
        crate::policy::PostPolicy::can_update(auth, existing)?;
        self.aspect_engine
            .before_update("posts", auth, existing, req)
            .await
    }

    async fn before_delete(
        &self,
        auth: &AuthUser,
        existing: &crate::models::post::Post,
    ) -> AppResult<crate::aspects::Dispatched> {
        crate::policy::PostPolicy::can_delete(auth, existing)?;
        self.aspect_engine
            .before_delete("posts", auth, existing)
            .await
    }

    fn after_created(&self, resp: &PostResponse) {
        self.aspect_engine.emit(Event::PostCreated(resp.clone()));
    }

    fn after_updated(&self, existing: &crate::models::post::Post) {
        self.aspect_engine
            .emit(Event::PostUpdated(existing.clone()));
    }

    fn after_deleted(&self, existing: &crate::models::post::Post) {
        self.aspect_engine
            .emit(Event::PostDeleted(existing.clone()));
    }
}

#[async_trait]
impl PostService for PostServiceImpl {
    #[tracing::instrument(skip(self), fields(slug = tracing::field::Empty))]
    async fn create(&self, auth: &AuthUser, req: CreatePostRequest) -> AppResult<PostResponse> {
        let (req, d) = self.before_create(auth, req).await?;
        let status = req.status.unwrap_or(PostStatus::Draft);

        let slug = d.str_or("slug", || slug_aspect::make_unique_slug(&req.title));
        let excerpt = d.str_or("excerpt", || {
            crate::aspects::excerpt_aspect::extract_excerpt(&req.content, 200)
        });

        let created_by = auth.user_id().ok_or(AppError::Unauthorized)?;
        let author_name =
            crate::models::post::get_author_name(&self.pool, created_by, auth.tenant_id())
                .await
                .inspect_err(|e| tracing::warn!("failed to fetch author name: {e}"))
                .ok()
                .flatten();

        let category_id = if let Some(ref raw_id) = req.category_id {
            let parsed = crate::types::snowflake_id::parse_id(raw_id)?;
            raisfast_derive::crud_resolve_id!(&self.pool, "categories", *parsed, tenant: auth.tenant_id())?
        } else {
            None
        };
        let category_name = if let Some(cat_id) = category_id {
            crate::models::post::get_category_name(
                &self.pool,
                SnowflakeId(cat_id),
                auth.tenant_id(),
            )
            .await
            .inspect_err(|e| tracing::warn!("failed to fetch category name: {e}"))
            .ok()
            .flatten()
        } else {
            None
        };

        let tag_ids = match req.tag_ids {
            Some(ref ids) => {
                let mut resolved = Vec::new();
                for raw_id in ids {
                    let parsed = crate::types::snowflake_id::parse_id(raw_id)?;
                    if let Some(int_id) = raisfast_derive::crud_resolve_id!(&self.pool, "tags", *parsed, tenant: auth.tenant_id())?
                    {
                        resolved.push(int_id);
                    }
                }
                Some(resolved)
            }
            None => None,
        };
        let tags = if let Some(ref ids) = tag_ids {
            crate::models::post::get_tags_by_ids(&self.pool, ids, auth.tenant_id())
                .await
                .inspect_err(|e| tracing::warn!("failed to fetch tags: {e}"))
                .unwrap_or_default()
        } else {
            Vec::new()
        };

        let cmd = CreatePostCmd {
            title: req.title,
            slug,
            content: req.content,
            excerpt: Some(excerpt),
            cover_image: req.cover_image,
            image_ids: req.image_ids,
            status,
            created_by,
            updated_by: auth.user_id(),
            category_id,
            tag_ids,
            meta_title: req.meta_title,
            meta_description: req.meta_description,
            og_title: req.og_title,
            og_description: req.og_description,
            og_image: req.og_image,
            canonical_url: req.canonical_url,
        };
        let p = create_post_with_tags(&self.pool, cmd, auth.tenant_id()).await?;

        let resp = build_response_from_post(&p, author_name, category_name, tags).await?;
        tracing::Span::current().record("slug", &resp.slug);
        self.after_created(&resp);
        Ok(resp)
    }

    async fn update(
        &self,
        auth: &AuthUser,
        slug: &str,
        req: UpdatePostRequest,
    ) -> AppResult<PostResponse> {
        let existing = crate::models::post::find_by_slug(&self.pool, slug, auth.tenant_id())
            .await?
            .ok_or_else(|| AppError::not_found("post"))?;

        let existing_id = existing.id;
        let (req, d) = self.before_update(auth, &existing, req).await?;
        let resp = self
            .update_inner(existing_id, existing, req, d, auth)
            .await?;
        let updated = crate::models::post::find_by_id(&self.pool, existing_id, auth.tenant_id())
            .await?
            .ok_or_else(|| AppError::not_found("post"))?;
        self.after_updated(&updated);
        Ok(resp)
    }

    async fn delete(&self, auth: &AuthUser, slug: &str) -> AppResult<()> {
        let existing = crate::models::post::find_by_slug(&self.pool, slug, auth.tenant_id())
            .await?
            .ok_or_else(|| AppError::not_found("post"))?;

        self.before_delete(auth, &existing).await?;

        crate::models::post::delete(&self.pool, existing.id, auth.tenant_id()).await?;
        self.after_deleted(&existing);
        Ok(())
    }

    async fn get(&self, auth: &AuthUser, slug: &str) -> AppResult<PostResponse> {
        let row =
            crate::models::post::increment_view_count_joined(&self.pool, slug, auth.tenant_id())
                .await?;
        let tags = crate::models::post::get_post_tags(&self.pool, row.id, auth.tenant_id())
            .await
            .unwrap_or_default();
        joined_row_to_response(row, tags).await
    }

    async fn get_any_status(&self, auth: &AuthUser, slug: &str) -> AppResult<PostResponse> {
        let post = crate::models::post::find_by_slug(&self.pool, slug, auth.tenant_id()).await?;
        let post = post.ok_or_else(|| AppError::not_found("post not found"))?;
        let row =
            crate::models::post::find_joined_by_id(&self.pool, post.id, auth.tenant_id()).await?;
        let tags = crate::models::post::get_post_tags(&self.pool, row.id, auth.tenant_id())
            .await
            .unwrap_or_default();
        joined_row_to_response(row, tags).await
    }

    async fn admin_get_by_id(&self, auth: &AuthUser, id: SnowflakeId) -> AppResult<PostResponse> {
        let row = crate::models::post::find_joined_by_id(&self.pool, id, auth.tenant_id()).await?;
        let tags = crate::models::post::get_post_tags(&self.pool, row.id, auth.tenant_id())
            .await
            .unwrap_or_default();
        joined_row_to_response(row, tags).await
    }

    #[allow(clippy::too_many_arguments)]
    async fn list(
        &self,
        auth: &AuthUser,
        page: i64,
        page_size: i64,
        category_id: Option<i64>,
        tag_id: Option<i64>,
        q: Option<&str>,
    ) -> AppResult<(Vec<PostResponse>, i64)> {
        list_posts_inner(
            &self.pool,
            page,
            page_size,
            category_id,
            tag_id,
            q,
            Some(self.search.as_ref()),
            auth,
        )
        .await
    }

    async fn list_all(
        &self,
        auth: &AuthUser,
        page: i64,
        page_size: i64,
        status: Option<PostStatus>,
        keyword: Option<&str>,
    ) -> AppResult<(Vec<PostResponse>, i64)> {
        list_all_posts_inner(&self.pool, page, page_size, status, keyword, auth).await
    }

    async fn admin_update(
        &self,
        auth: &AuthUser,
        id: SnowflakeId,
        req: UpdatePostRequest,
    ) -> AppResult<PostResponse> {
        let existing = crate::models::post::find_by_id(&self.pool, id, auth.tenant_id())
            .await?
            .ok_or_else(|| AppError::not_found("post"))?;

        let (req, d) = self.before_update(auth, &existing, req).await?;
        let resp = self
            .update_inner(id, existing.clone(), req, d, auth)
            .await?;
        let updated = crate::models::post::find_by_id(&self.pool, id, auth.tenant_id())
            .await?
            .unwrap_or(existing);
        self.after_updated(&updated);
        Ok(resp)
    }

    async fn admin_delete(&self, auth: &AuthUser, id: SnowflakeId) -> AppResult<()> {
        let existing = crate::models::post::find_by_id(&self.pool, id, auth.tenant_id())
            .await?
            .ok_or_else(|| AppError::not_found("post"))?;

        self.before_delete(auth, &existing).await?;

        crate::models::post::delete(&self.pool, id, auth.tenant_id()).await?;
        self.after_deleted(&existing);
        Ok(())
    }

    async fn batch(&self, auth: &AuthUser, action: &str, ids: &[String]) -> AppResult<usize> {
        let mut affected = 0usize;
        for raw_id in ids {
            let Ok(parsed_id) = crate::types::snowflake_id::parse_id(raw_id) else {
                continue;
            };
            let Some(int_id) = raisfast_derive::crud_resolve_id!(&self.pool, "posts", *parsed_id, tenant: auth.tenant_id())?
            else {
                continue;
            };

            match action {
                "delete" => {
                    let Ok(Some(existing)) = crate::models::post::find_by_id(
                        &self.pool,
                        SnowflakeId(int_id),
                        auth.tenant_id(),
                    )
                    .await
                    else {
                        continue;
                    };
                    if self.before_delete(auth, &existing).await.is_err() {
                        continue;
                    }
                    if crate::models::post::delete(
                        &self.pool,
                        SnowflakeId(int_id),
                        auth.tenant_id(),
                    )
                    .await
                    .is_ok()
                    {
                        self.after_deleted(&existing);
                        affected += 1;
                    }
                }
                "publish" | "unpublish" => {
                    let status = if action == "publish" {
                        PostStatus::Published
                    } else {
                        PostStatus::Draft
                    };
                    if let Some(post) = crate::models::post::find_by_id(
                        &self.pool,
                        SnowflakeId(int_id),
                        auth.tenant_id(),
                    )
                    .await?
                    {
                        let (req, _d) = self
                            .before_update(
                                auth,
                                &post,
                                UpdatePostRequest {
                                    title: None,
                                    slug: None,
                                    content: None,
                                    excerpt: None,
                                    cover_image: None,
                                    image_ids: None,
                                    status: Some(status),
                                    category_id: None,
                                    tag_ids: None,
                                    meta_title: None,
                                    meta_description: None,
                                    og_title: None,
                                    og_description: None,
                                    og_image: None,
                                    canonical_url: None,
                                },
                            )
                            .await?;
                        let cmd = UpdatePostCmd {
                            id: post.id,
                            title: None,
                            slug: None,
                            content: None,
                            excerpt: None,
                            cover_image: None,
                            image_ids: None,
                            status: req.status,
                            category_id: None,
                            tag_ids: None,
                            updated_by: auth.user_id(),
                            meta_title: None,
                            meta_description: None,
                            og_title: None,
                            og_description: None,
                            og_image: None,
                            canonical_url: None,
                        };
                        if update_post_with_tags(&self.pool, cmd, auth.tenant_id())
                            .await
                            .is_ok()
                        {
                            self.after_updated(&post);
                            affected += 1;
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(affected)
    }

    async fn list_recent_published(
        &self,
        limit: i64,
        tenant_id: Option<&str>,
    ) -> AppResult<Vec<PostJoinedRow>> {
        let (rows, _) = crate::models::post::find_published_joined(
            &self.pool, 1, limit, None, None, None, tenant_id,
        )
        .await?;
        Ok(rows)
    }
}

impl PostServiceImpl {
    async fn update_inner(
        &self,
        id: SnowflakeId,
        existing: crate::models::post::Post,
        req: UpdatePostRequest,
        mut d: crate::aspects::Dispatched,
        auth: &AuthUser,
    ) -> AppResult<PostResponse> {
        let content = req.content.as_deref().unwrap_or(&existing.content);
        if let Some(ref slug_val) = req.slug {
            if !slug_val.is_empty() && slug_val != &existing.slug {
                AspectEngine::set_dispatched_field(
                    &mut d,
                    "slug",
                    slug_aspect::make_unique_slug(slug_val),
                );
            }
        } else if let Some(ref title_val) = req.title {
            let slugified = slugify(title_val);
            if slugified != existing.slug {
                AspectEngine::set_dispatched_field(
                    &mut d,
                    "slug",
                    slug_aspect::make_unique_slug(&slugified),
                );
            }
        }

        let slug = d.str("slug");
        let excerpt = d.str_or("excerpt", || {
            crate::aspects::excerpt_aspect::extract_excerpt(content, 200)
        });

        let category_id = if let Some(ref raw_id) = req.category_id {
            let parsed = crate::types::snowflake_id::parse_id(raw_id)?;
            raisfast_derive::crud_resolve_id!(&self.pool, "categories", *parsed, tenant: auth.tenant_id())?
        } else {
            None
        };
        let tag_ids = match req.tag_ids {
            Some(ref ids) => {
                let mut resolved = Vec::new();
                for raw_id in ids {
                    let parsed = crate::types::snowflake_id::parse_id(raw_id)?;
                    if let Some(int_id) = raisfast_derive::crud_resolve_id!(&self.pool, "tags", *parsed, tenant: auth.tenant_id())?
                    {
                        resolved.push(int_id);
                    }
                }
                Some(resolved)
            }
            None => None,
        };

        let cmd = UpdatePostCmd {
            id,
            title: req.title,
            slug,
            content: Some(content.to_string()),
            excerpt: Some(excerpt),
            cover_image: req.cover_image,
            image_ids: req.image_ids,
            status: req.status,
            category_id,
            tag_ids,
            updated_by: auth.user_id(),
            meta_title: req.meta_title,
            meta_description: req.meta_description,
            og_title: req.og_title,
            og_description: req.og_description,
            og_image: req.og_image,
            canonical_url: req.canonical_url,
        };
        update_post_with_tags(&self.pool, cmd, auth.tenant_id()).await?;

        build_post_response_from_pool(&self.pool, id, auth).await
    }
}

// ─── Private helpers ───

async fn joined_row_to_response(
    r: PostJoinedRow,
    tags: Vec<crate::models::post::TagBrief>,
) -> AppResult<PostResponse> {
    let status = r.status;
    let comment_status = r.comment_status;
    Ok(PostResponse {
        id: r.id.to_string(),
        title: r.title,
        slug: r.slug,
        content: r.content,
        excerpt: r.excerpt,
        cover_image: r.cover_image,
        image_ids: r.image_ids,
        status,
        created_by: None,
        author_name: r.author_name,
        category_id: None,
        category_name: r.category_name,
        tags,
        view_count: r.view_count,
        is_pinned: r.is_pinned,
        password: r.password,
        comment_status,
        format: r.format,
        template: r.template,
        meta_title: r.meta_title,
        meta_description: r.meta_description,
        og_title: r.og_title,
        og_description: r.og_description,
        og_image: r.og_image,
        canonical_url: r.canonical_url,
        reading_time: r.reading_time,
        created_at: r.created_at,
        updated_at: r.updated_at,
        published_at: r.published_at,
        title_highlight: None,
        excerpt_highlight: None,
    })
}

async fn build_response_from_post(
    post: &crate::models::post::Post,
    author_name: Option<String>,
    category_name: Option<String>,
    tags: Vec<crate::models::post::TagBrief>,
) -> AppResult<PostResponse> {
    let status = post.status;
    let comment_status = post.comment_status;
    Ok(PostResponse {
        id: post.id.to_string(),
        title: post.title.clone(),
        slug: post.slug.clone(),
        content: post.content.clone(),
        excerpt: post.excerpt.clone(),
        cover_image: post.cover_image.clone(),
        image_ids: post.image_ids.clone(),
        status,
        created_by: None,
        author_name,
        category_id: None,
        category_name,
        tags,
        view_count: post.view_count,
        is_pinned: post.is_pinned,
        password: post.password.clone(),
        comment_status,
        format: post.format.clone(),
        template: post.template.clone(),
        meta_title: post.meta_title.clone(),
        meta_description: post.meta_description.clone(),
        og_title: post.og_title.clone(),
        og_description: post.og_description.clone(),
        og_image: post.og_image.clone(),
        canonical_url: post.canonical_url.clone(),
        reading_time: post.reading_time,
        created_at: post.created_at,
        updated_at: post.updated_at,
        published_at: post.published_at,
        title_highlight: None,
        excerpt_highlight: None,
    })
}

async fn build_post_response_from_pool(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    auth: &AuthUser,
) -> AppResult<PostResponse> {
    let row = crate::models::post::find_joined_by_id(pool, id, auth.tenant_id()).await?;
    let tags = crate::models::post::get_post_tags(pool, row.id, auth.tenant_id())
        .await
        .unwrap_or_default();
    joined_row_to_response(row, tags).await
}

#[allow(clippy::too_many_arguments)]
async fn list_posts_inner(
    pool: &crate::db::Pool,
    page: i64,
    page_size: i64,
    category_id: Option<i64>,
    tag_id: Option<i64>,
    q: Option<&str>,
    search: Option<&dyn SearchEngine>,
    auth: &AuthUser,
) -> AppResult<(Vec<PostResponse>, i64)> {
    let (rows, total, highlights): (Vec<_>, _, std::collections::HashMap<SnowflakeId, _>) =
        if let (Some(engine), Some(keyword)) = (search, q) {
            if !engine.is_noop() && !keyword.is_empty() {
                let (results, total) = engine.search(keyword, page, page_size).await?;
                let mut hmap: std::collections::HashMap<SnowflakeId, _> =
                    std::collections::HashMap::new();
                let ids: Vec<i64> = results
                    .into_iter()
                    .filter_map(|r| {
                        let pid: i64 = r.post_id.parse().ok()?;
                        hmap.insert(SnowflakeId(pid), (r.title_highlight, r.excerpt_highlight));
                        Some(pid)
                    })
                    .collect();
                let rows =
                    crate::models::post::find_joined_by_ids(pool, &ids, auth.tenant_id()).await?;
                (rows, total, hmap)
            } else {
                let (rows, total) = crate::models::post::find_published_joined(
                    pool,
                    page,
                    page_size,
                    category_id,
                    tag_id,
                    if keyword.is_empty() {
                        None
                    } else {
                        Some(keyword)
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
            let (rows, total) = crate::models::post::find_published_joined(
                pool,
                page,
                page_size,
                category_id,
                tag_id,
                q,
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

    let post_ids: Vec<SnowflakeId> = rows.iter().map(|r: &PostJoinedRow| r.id).collect();
    let tags_map = crate::models::post::get_tags_for_posts(pool, &post_ids, auth.tenant_id())
        .await
        .unwrap_or_default();

    let mut responses = Vec::with_capacity(rows.len());
    for r in rows {
        let (title_hl, excerpt_hl): (Option<String>, Option<String>) = highlights
            .get(&r.id)
            .map_or((None, None), |(t, e)| (t.clone(), e.clone()));
        let status = r.status;
        let comment_status = r.comment_status;
        responses.push(PostResponse {
            id: r.id.to_string(),
            title: r.title,
            slug: r.slug,
            content: r.content,
            excerpt: r.excerpt,
            cover_image: r.cover_image,
            image_ids: r.image_ids,
            status,
            created_by: None,
            author_name: r.author_name,
            category_id: None,
            category_name: r.category_name,
            tags: tags_map.get(&r.id).cloned().unwrap_or_default(),
            view_count: r.view_count,
            is_pinned: r.is_pinned,
            password: r.password,
            comment_status,
            format: r.format,
            template: r.template,
            meta_title: r.meta_title,
            meta_description: r.meta_description,
            og_title: r.og_title,
            og_description: r.og_description,
            og_image: r.og_image,
            canonical_url: r.canonical_url,
            reading_time: r.reading_time,
            created_at: r.created_at,
            updated_at: r.updated_at,
            published_at: r.published_at,
            title_highlight: title_hl,
            excerpt_highlight: excerpt_hl,
        });
    }

    Ok((responses, total))
}

async fn list_all_posts_inner(
    pool: &crate::db::Pool,
    page: i64,
    page_size: i64,
    status: Option<PostStatus>,
    keyword: Option<&str>,
    auth: &AuthUser,
) -> AppResult<(Vec<PostResponse>, i64)> {
    let (rows, total) = crate::models::post::find_all_joined(
        pool,
        page,
        page_size,
        status,
        keyword,
        auth.tenant_id(),
    )
    .await?;

    let post_ids: Vec<SnowflakeId> = rows.iter().map(|r| r.id).collect();
    let tags_map = crate::models::post::get_tags_for_posts(pool, &post_ids, auth.tenant_id())
        .await
        .unwrap_or_default();

    let mut responses = Vec::with_capacity(rows.len());
    for r in rows {
        let status = r.status;
        let comment_status = r.comment_status;
        responses.push(PostResponse {
            id: r.id.to_string(),
            title: r.title,
            slug: r.slug,
            content: r.content,
            excerpt: r.excerpt,
            cover_image: r.cover_image,
            image_ids: r.image_ids,
            status,
            created_by: None,
            author_name: r.author_name,
            category_id: None,
            category_name: r.category_name,
            tags: tags_map.get(&r.id).cloned().unwrap_or_default(),
            view_count: r.view_count,
            is_pinned: r.is_pinned,
            password: r.password,
            comment_status,
            format: r.format,
            template: r.template,
            meta_title: r.meta_title,
            meta_description: r.meta_description,
            og_title: r.og_title,
            og_description: r.og_description,
            og_image: r.og_image,
            canonical_url: r.canonical_url,
            reading_time: r.reading_time,
            created_at: r.created_at,
            updated_at: r.updated_at,
            published_at: r.published_at,
            title_highlight: None,
            excerpt_highlight: None,
        });
    }

    Ok((responses, total))
}

// ─── Helpers for create/update with tag syncing ───

async fn create_post_with_tags(
    pool: &crate::db::Pool,
    cmd: CreatePostCmd,
    tenant_id: Option<&str>,
) -> AppResult<crate::models::post::Post> {
    if let Some(ref tag_ids) = cmd.tag_ids {
        let _guard = crate::db::connection::acquire_write().await;
        let mut tx = pool.begin().await?;
        let p = crate::models::post::create_tx(&mut tx, &cmd, tenant_id).await?;
        crate::models::tagging::sync_tags_tx(&mut tx, "post", p.id, tag_ids, tenant_id).await?;
        tx.commit().await?;
        Ok(p)
    } else {
        crate::models::post::create(pool, &cmd, tenant_id).await
    }
}

async fn update_post_with_tags(
    pool: &crate::db::Pool,
    cmd: UpdatePostCmd,
    tenant_id: Option<&str>,
) -> AppResult<crate::models::post::Post> {
    if let Some(ref tag_ids) = cmd.tag_ids {
        let _guard = crate::db::connection::acquire_write().await;
        let mut tx = pool.begin().await?;
        crate::models::post::update_tx(&mut tx, &cmd, tenant_id).await?;
        crate::models::tagging::sync_tags_tx(&mut tx, "post", cmd.id, tag_ids, tenant_id).await?;
        tx.commit().await?;
        crate::models::post::find_by_id(pool, cmd.id, tenant_id)
            .await?
            .ok_or_else(|| AppError::not_found("post"))
    } else {
        crate::models::post::update(pool, &cmd, tenant_id).await
    }
}

// ─── Tests ───

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_excerpt_short_content() {
        let content = "short";
        let result = crate::aspects::excerpt_aspect::extract_excerpt(content, 200);
        assert_eq!(result, "short");
    }

    #[test]
    fn extract_excerpt_truncates_long_content() {
        let content = "a".repeat(300);
        let result = crate::aspects::excerpt_aspect::extract_excerpt(&content, 200);
        assert!(result.ends_with("..."));
        assert_eq!(result.len(), 203);
    }

    #[test]
    fn extract_excerpt_exact_boundary() {
        let content = "a".repeat(200);
        let result = crate::aspects::excerpt_aspect::extract_excerpt(&content, 200);
        assert_eq!(result, "a".repeat(200));
    }

    #[test]
    fn extract_excerpt_unicode_safe() {
        let content = "你好世界".repeat(100);
        let result = crate::aspects::excerpt_aspect::extract_excerpt(&content, 200);
        assert!(result.ends_with("...") || result.len() <= 200);
    }

    #[test]
    fn make_unique_slug_has_suffix() {
        let slug = slug_aspect::make_unique_slug("my-post");
        assert!(slug.starts_with("my-post-"));
        let suffix = &slug["my-post-".len()..];
        assert_eq!(suffix.len(), 4);
        assert!(suffix.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn make_unique_slug_different_each_call() {
        let s1 = slug_aspect::make_unique_slug("test");
        let s2 = slug_aspect::make_unique_slug("test");
        assert_ne!(s1, s2);
    }

    #[test]
    fn extract_excerpt_zero_max_len() {
        let content = "hello world";
        let result = crate::aspects::excerpt_aspect::extract_excerpt(content, 0);
        assert!(result.is_empty() || result.ends_with("..."));
    }

    #[tokio::test]
    async fn resolve_id_numeric_no_tenant() {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO users (id, username, role, status, registered_via) VALUES (42, 'u', 'user', 'active', 'email')")
            .execute(&pool)
            .await
            .unwrap();
        let result = raisfast_derive::crud_resolve_id!(&pool, "users", *SnowflakeId(42));
        assert_eq!(result.unwrap(), Some(42));
    }

    #[tokio::test]
    async fn resolve_id_unsafe_table() {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        let result = raisfast_derive::crud_resolve_id!(&pool, "drop table", *SnowflakeId(999));
        assert_eq!(result.unwrap(), None);
    }

    #[tokio::test]
    async fn resolve_id_with_tenant() {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO users (id, username, role, status, registered_via) VALUES (1, 'user1', 'user', 'active', 'email')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO posts (id, title, slug, content, status, created_by, updated_by, tenant_id) VALUES (1, 'T', 't', 'c', 'draft', 1, 1, 't1')")
            .execute(&pool)
            .await
            .unwrap();
        let result =
            raisfast_derive::crud_resolve_id!(&pool, "posts", *SnowflakeId(1), tenant: Some("t1"))
                .unwrap();
        assert_eq!(result, Some(1));
    }

    #[tokio::test]
    async fn resolve_id_with_wrong_tenant() {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO users (id, username, role, status, registered_via) VALUES (1, 'user1', 'user', 'active', 'email')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO posts (id, title, slug, content, status, created_by, updated_by, tenant_id) VALUES (1, 'T', 't', 'c', 'draft', 1, 1, 't1')")
            .execute(&pool)
            .await
            .unwrap();
        let result = raisfast_derive::crud_resolve_id!(&pool, "posts", *SnowflakeId(1), tenant: Some("wrong"))
            .unwrap();
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn resolve_id_not_found_with_tenant() {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        let result = raisfast_derive::crud_resolve_id!(&pool, "users", *SnowflakeId(99999), tenant: Some("t1"))
            .unwrap();
        assert_eq!(result, None);
    }
}
