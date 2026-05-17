//! Comment model and database queries
//!
//! Defines data structures related to comments, including the full row model,
//! response models supporting nested tree structures, request validation structs,
//! and CRUD operations on the `comments` table.
//!
//! Comments support multi-level nested replies via `parent_id` to build parent-child
//! relationships. The tree structure is converted from a flat list by the [`build_tree`]
//! function, and nesting depth is limited to a maximum of 3 levels by [`validate_depth`].

use serde::{Deserialize, Serialize};
use sqlx::FromRow;
#[cfg(feature = "export-types")]
use ts_rs::TS;

use crate::db::dialect::ph;
use crate::db::tenant::{tenant_filter_aliased_ph, tenant_filter_ph};
use crate::errors::app_error::{AppError, AppResult};
use crate::utils::tz::Timestamp;

define_enum!(
    CommentStatus {
        Pending = "pending",
        Approved = "approved",
        Spam = "spam",
    }
);

#[derive(Debug, Serialize, Deserialize, Clone)]
#[non_exhaustive]
pub struct Comment {
    pub id: i64,
    pub document_id: String,
    pub tenant_id: Option<String>,
    pub post_id: i64,
    pub created_by: Option<i64>,
    pub updated_by: Option<i64>,
    pub nickname: Option<String>,
    pub email: Option<String>,
    pub content: String,
    pub parent_id: Option<i64>,
    pub author_ip: Option<String>,
    pub author_url: Option<String>,
    pub status: CommentStatus,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

crate::impl_from_row_opt_tenant!(Comment {
    required { id, document_id, post_id, content, status, created_at, updated_at }
    optional { created_by, updated_by, nickname, email, parent_id, author_ip, author_url }
});

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, Clone)]
#[non_exhaustive]
pub struct CommentResponse {
    pub id: i64,
    pub document_id: String,
    pub post_id: i64,
    pub created_by: Option<i64>,
    pub nickname: Option<String>,
    pub content: String,
    pub parent_id: Option<i64>,
    pub depth: i32,
    pub replies: Vec<CommentResponse>,
    pub created_at: Timestamp,
}

pub async fn find_by_id(
    pool: &crate::db::Pool,
    id: i64,
    tenant_id: Option<&str>,
) -> AppResult<Option<Comment>> {
    Ok(tenant_find!(pool, "comments" => Comment, "id" => id, tenant_id)?)
}

pub async fn find_by_document_id(
    pool: &crate::db::Pool,
    document_id: &str,
    tenant_id: Option<&str>,
) -> AppResult<Option<Comment>> {
    Ok(tenant_find!(pool, "comments" => Comment, "document_id" => document_id, tenant_id)?)
}

pub async fn create(
    pool: &crate::db::Pool,
    cmd: &crate::commands::CreateCommentCmd,
    tenant_id: Option<&str>,
) -> AppResult<Comment> {
    let (document_id, now) = crate::utils::id::new_document_id_and_timestamp();

    tenant_insert!(
        pool,
        "comments",
        [
            "document_id" => &document_id,
            "post_id" => cmd.post_id,
            "created_by" => cmd.created_by,
            "updated_by" => cmd.created_by,
            "nickname" => &cmd.nickname,
            "email" => &cmd.email,
            "content" => &cmd.content,
            "parent_id" => cmd.parent_id,
            "created_at" => now,
            "updated_at" => now,
            "status" => CommentStatus::Pending
        ],
        tenant_id
    )?;

    find_by_document_id(pool, &document_id, tenant_id)
        .await?
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("failed to fetch created comment")))
}

pub async fn find_approved_by_post(
    pool: &crate::db::Pool,
    post_id: i64,
    tenant_id: Option<&str>,
) -> AppResult<Vec<Comment>> {
    let filter = tenant_filter_ph(tenant_id, 3);
    let sql = format!(
        "SELECT * FROM comments WHERE post_id = {} AND status = {}{filter} ORDER BY created_at ASC",
        ph(1),
        ph(2)
    );
    let comments = tenant_query!(
        pool,
        Comment,
        &sql,
        [post_id, CommentStatus::Approved],
        tenant_id,
        fetch_all
    )?;
    Ok(comments)
}

pub async fn find_approved_by_post_paginated(
    pool: &crate::db::Pool,
    post_id: i64,
    page: i64,
    page_size: i64,
    tenant_id: Option<&str>,
) -> AppResult<(Vec<Comment>, i64)> {
    check_schema!("comments", "post_id", "status", "created_at");
    let offset = (page - 1) * page_size;
    let filter = tenant_filter_ph(tenant_id, 3);
    let base = usize::from(tenant_id.is_some());
    let sql = format!(
        "SELECT * FROM comments WHERE post_id = {} AND status = {}{filter} ORDER BY created_at ASC LIMIT {} OFFSET {}",
        ph(1),
        ph(2),
        ph(base + 3),
        ph(base + 4)
    );
    let mut q = sqlx::query_as::<_, Comment>(&sql)
        .bind(post_id)
        .bind(CommentStatus::Approved);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    q = q.bind(page_size).bind(offset);
    let comments = q.fetch_all(pool).await?;

    let filter2 = tenant_filter_ph(tenant_id, 3);
    let sql2 = format!(
        "SELECT COUNT(*) FROM comments WHERE post_id = {} AND status = {}{filter2}",
        ph(1),
        ph(2)
    );
    let mut q2 = sqlx::query_scalar::<_, i64>(&sql2)
        .bind(post_id)
        .bind(CommentStatus::Approved);
    if let Some(tid) = tenant_id {
        q2 = q2.bind(tid);
    }
    let total: i64 = q2.fetch_one(pool).await?;

    Ok((comments, total))
}

pub async fn find_all_by_post(
    pool: &crate::db::Pool,
    post_id: i64,
    tenant_id: Option<&str>,
) -> AppResult<Vec<Comment>> {
    let filter = tenant_filter_ph(tenant_id, 2);
    let sql = format!(
        "SELECT * FROM comments WHERE post_id = {}{filter} ORDER BY created_at ASC",
        ph(1)
    );
    let comments = tenant_query!(pool, Comment, &sql, [post_id], tenant_id, fetch_all)?;
    Ok(comments)
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, FromRow, Serialize, Clone)]
pub struct AdminCommentRow {
    pub id: i64,
    pub post_id: i64,
    pub post_title: String,
    pub created_by: Option<i64>,
    pub nickname: Option<String>,
    pub email: Option<String>,
    pub content: String,
    pub parent_id: Option<i64>,
    pub status: CommentStatus,
    pub created_at: Timestamp,
}

pub async fn find_all_paginated(
    pool: &crate::db::Pool,
    page: i64,
    page_size: i64,
    tenant_id: Option<&str>,
) -> AppResult<(Vec<AdminCommentRow>, i64)> {
    check_schema!(
        "comments",
        "id",
        "post_id",
        "created_by",
        "nickname",
        "email",
        "content",
        "parent_id",
        "status",
        "created_at"
    );
    check_schema!("posts", "id", "title");
    let offset = (page - 1) * page_size;
    let filter = tenant_filter_aliased_ph("c", tenant_id, 1);
    let base = usize::from(tenant_id.is_some());
    let sql = format!(
        "SELECT c.id, c.post_id, p.title AS post_title, c.created_by, c.nickname, c.email, c.content, c.parent_id, c.status, c.created_at FROM comments c JOIN posts p ON c.post_id = p.id WHERE 1=1{filter} ORDER BY c.created_at DESC LIMIT {} OFFSET {}",
        ph(base + 1),
        ph(base + 2)
    );
    let mut q = sqlx::query_as::<_, AdminCommentRow>(&sql);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    q = q.bind(page_size).bind(offset);
    let rows = q.fetch_all(pool).await?;

    let filter2 = tenant_filter_ph(tenant_id, 1);
    let sql2 = format!("SELECT COUNT(*) FROM comments WHERE 1=1{filter2}");
    let mut q2 = sqlx::query_scalar::<_, i64>(&sql2);
    if let Some(tid) = tenant_id {
        q2 = q2.bind(tid);
    }
    let total: i64 = q2.fetch_one(pool).await?;

    Ok((rows, total))
}

pub async fn update_status(
    pool: &crate::db::Pool,
    id: i64,
    status: CommentStatus,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    let now = crate::utils::tz::now_utc();
    let result = tenant_update!(pool, "comments",
        bind: ["status" => status, "updated_at" => &now],
        where: "id" => id,
        tenant: tenant_id
    )?;

    AppError::expect_affected(&result, "comment")
}

pub async fn delete(pool: &crate::db::Pool, id: i64, tenant_id: Option<&str>) -> AppResult<()> {
    let result = tenant_delete!(pool, "comments", "id" => id, tenant_id)?;
    AppError::expect_affected(&result, "comment")
}

fn get_depth(comments: &[Comment], comment: &Comment) -> i32 {
    let mut depth = 0;
    let mut current_parent = comment.parent_id;
    let mut visited = std::collections::HashSet::new();
    while let Some(pid) = current_parent {
        if visited.contains(&pid) || depth > 10 {
            break;
        }
        visited.insert(pid);
        depth += 1;
        current_parent = comments
            .iter()
            .find(|c| c.id == pid)
            .and_then(|c| c.parent_id);
    }
    depth
}

#[must_use]
pub fn build_tree(comments: &[Comment]) -> Vec<CommentResponse> {
    let map: std::collections::HashMap<i64, Vec<Comment>> =
        comments
            .iter()
            .fold(std::collections::HashMap::new(), |mut acc, c| {
                let key = c.parent_id.unwrap_or_default();
                acc.entry(key).or_default().push(c.clone());
                acc
            });

    fn build(
        parent_id: i64,
        map: &std::collections::HashMap<i64, Vec<Comment>>,
        comments: &[Comment],
    ) -> Vec<CommentResponse> {
        map.get(&parent_id)
            .map(|children| {
                children
                    .iter()
                    .map(|c| {
                        let depth = get_depth(comments, c);
                        let replies = build(c.id, map, comments);
                        CommentResponse {
                            id: c.id,
                            document_id: c.document_id.clone(),
                            post_id: c.post_id,
                            created_by: c.created_by,
                            nickname: c.nickname.clone(),
                            content: c.content.clone(),
                            parent_id: c.parent_id,
                            depth,
                            replies,
                            created_at: c.created_at,
                        }
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    build(0, &map, comments)
}

const MAX_DEPTH: i32 = 3;

pub fn validate_depth(comments: &[Comment], parent_id: i64) -> AppResult<()> {
    let parent = comments
        .iter()
        .find(|c| c.id == parent_id)
        .ok_or_else(|| AppError::not_found("parent comment"))?;

    let depth = get_depth(comments, parent);
    if depth >= MAX_DEPTH {
        return Err(AppError::BadRequest("comment_depth".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_comment(id: i64, post_id: i64, parent_id: Option<i64>) -> Comment {
        Comment {
            id,
            tenant_id: Some(crate::constants::DEFAULT_TENANT.to_string()),
            document_id: format!("doc-{id}"),
            post_id,
            created_by: None,
            updated_by: None,
            nickname: None,
            email: None,
            content: "test".to_string(),
            parent_id,
            author_ip: None,
            author_url: None,
            status: CommentStatus::Approved,
            created_at: "2025-01-01T00:00:00Z".parse().unwrap(),
            updated_at: "2025-01-01T00:00:00Z".parse().unwrap(),
        }
    }

    #[test]
    fn build_tree_flat_comments() {
        let comments = vec![make_comment(1, 10, None), make_comment(2, 10, None)];
        let tree = build_tree(&comments);
        assert_eq!(tree.len(), 2);
        assert!(tree[0].replies.is_empty());
        assert!(tree[1].replies.is_empty());
    }

    #[test]
    fn build_tree_nested() {
        let comments = vec![
            make_comment(1, 10, None),
            make_comment(2, 10, Some(1)),
            make_comment(3, 10, Some(2)),
        ];
        let tree = build_tree(&comments);
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].id, 1);
        assert_eq!(tree[0].replies.len(), 1);
        assert_eq!(tree[0].replies[0].id, 2);
        assert_eq!(tree[0].replies[0].replies.len(), 1);
        assert_eq!(tree[0].replies[0].replies[0].id, 3);
    }

    #[test]
    fn build_tree_depth_values() {
        let comments = vec![
            make_comment(1, 10, None),
            make_comment(2, 10, Some(1)),
            make_comment(3, 10, Some(2)),
        ];
        let tree = build_tree(&comments);
        assert_eq!(tree[0].depth, 0);
        assert_eq!(tree[0].replies[0].depth, 1);
        assert_eq!(tree[0].replies[0].replies[0].depth, 2);
    }

    #[test]
    fn validate_depth_ok_within_limit() {
        let comments = vec![make_comment(1, 10, None), make_comment(2, 10, Some(1))];
        assert!(validate_depth(&comments, 2).is_ok());
    }

    #[test]
    fn validate_depth_fails_at_max() {
        let comments = vec![
            make_comment(1, 10, None),
            make_comment(2, 10, Some(1)),
            make_comment(3, 10, Some(2)),
            make_comment(4, 10, Some(3)),
        ];
        assert!(validate_depth(&comments, 4).is_err());
    }

    #[test]
    fn validate_depth_missing_parent() {
        let comments = vec![make_comment(1, 10, None)];
        assert!(validate_depth(&comments, 999).is_err());
    }

    mod integration {
        use super::*;
        use crate::commands::CreateCommentCmd;

        async fn setup_pool() -> crate::db::Pool {
            crate::test_pool!()
        }

        async fn insert_user(pool: &crate::db::Pool) -> i64 {
            let user = crate::models::user::create(
                pool,
                &crate::commands::user::CreateUserCmd {
                    username: "testuser".to_string(),
                    registered_via: crate::models::user::RegisteredVia::Email,
                },
                None,
            )
            .await
            .unwrap();
            user.id
        }

        async fn insert_post(pool: &crate::db::Pool, user_id: i64) -> i64 {
            let doc_id = crate::utils::id::new_document_id();
            let slug = format!("slug-{doc_id}");
            sqlx::query(
                "INSERT INTO posts (document_id, title, slug, content, status, created_by, updated_by) VALUES (?, 'Test', ?, 'content', 'published', ?, ?)",
            )
            .bind(&doc_id)
            .bind(&slug)
            .bind(user_id)
            .bind(user_id)
            .execute(pool)
            .await
            .unwrap();

            let (id,): (i64,) = sqlx::query_as("SELECT id FROM posts WHERE document_id = ?")
                .bind(&doc_id)
                .fetch_one(pool)
                .await
                .unwrap();
            id
        }

        fn make_cmd(post_id: i64) -> CreateCommentCmd {
            CreateCommentCmd {
                post_id,
                created_by: None,
                nickname: Some("Alice".into()),
                email: Some("alice@test.com".into()),
                content: "hello".into(),
                parent_id: None,
            }
        }

        #[tokio::test]
        async fn create_and_find_by_id() {
            let pool = setup_pool().await;
            let uid = insert_user(&pool).await;
            let pid = insert_post(&pool, uid).await;
            let c = create(&pool, &make_cmd(pid), None).await.unwrap();
            assert_eq!(c.post_id, pid);
            assert_eq!(c.content, "hello");
            let found = super::find_by_id(&pool, c.id, None).await.unwrap().unwrap();
            assert_eq!(found.id, c.id);
            assert_eq!(found.document_id, c.document_id);
        }

        #[tokio::test]
        async fn find_by_document_id_test() {
            let pool = setup_pool().await;
            let uid = insert_user(&pool).await;
            let pid = insert_post(&pool, uid).await;
            let c = create(&pool, &make_cmd(pid), None).await.unwrap();
            let found = super::find_by_document_id(&pool, &c.document_id, None)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(found.id, c.id);
            assert_eq!(found.content, "hello");
        }

        #[tokio::test]
        async fn find_approved_by_post_returns_only_approved() {
            let pool = setup_pool().await;
            let uid = insert_user(&pool).await;
            let pid = insert_post(&pool, uid).await;
            let c1 = create(&pool, &make_cmd(pid), None).await.unwrap();
            let _c2 = create(&pool, &make_cmd(pid), None).await.unwrap();
            update_status(&pool, c1.id, CommentStatus::Approved, None)
                .await
                .unwrap();

            let approved = super::find_approved_by_post(&pool, pid, None)
                .await
                .unwrap();
            assert_eq!(approved.len(), 1);
            assert_eq!(approved[0].id, c1.id);
        }

        #[tokio::test]
        async fn find_approved_by_post_paginated_test() {
            let pool = setup_pool().await;
            let uid = insert_user(&pool).await;
            let pid = insert_post(&pool, uid).await;
            let mut ids = Vec::new();
            for i in 0..5 {
                let mut cmd = make_cmd(pid);
                cmd.content = format!("comment {i}");
                let c = create(&pool, &cmd, None).await.unwrap();
                update_status(&pool, c.id, CommentStatus::Approved, None)
                    .await
                    .unwrap();
                ids.push(c.id);
            }

            let (page1, total) = super::find_approved_by_post_paginated(&pool, pid, 1, 2, None)
                .await
                .unwrap();
            assert_eq!(total, 5);
            assert_eq!(page1.len(), 2);

            let (page3, _) = super::find_approved_by_post_paginated(&pool, pid, 3, 2, None)
                .await
                .unwrap();
            assert_eq!(page3.len(), 1);
        }

        #[tokio::test]
        async fn update_status_changes_status() {
            let pool = setup_pool().await;
            let uid = insert_user(&pool).await;
            let pid = insert_post(&pool, uid).await;
            let c = create(&pool, &make_cmd(pid), None).await.unwrap();
            assert_eq!(c.status, CommentStatus::Pending);
            update_status(&pool, c.id, CommentStatus::Approved, None)
                .await
                .unwrap();
            let found = super::find_by_id(&pool, c.id, None).await.unwrap().unwrap();
            assert_eq!(found.status, CommentStatus::Approved);
        }

        #[tokio::test]
        async fn delete_removes_comment() {
            let pool = setup_pool().await;
            let uid = insert_user(&pool).await;
            let pid = insert_post(&pool, uid).await;
            let c = create(&pool, &make_cmd(pid), None).await.unwrap();
            super::delete(&pool, c.id, None).await.unwrap();
            let found = super::find_by_id(&pool, c.id, None).await.unwrap();
            assert!(found.is_none());
        }

        #[tokio::test]
        async fn find_all_paginated_test() {
            let pool = setup_pool().await;
            let uid = insert_user(&pool).await;
            let pid = insert_post(&pool, uid).await;
            for i in 0..5 {
                let mut cmd = make_cmd(pid);
                cmd.content = format!("comment {i}");
                create(&pool, &cmd, None).await.unwrap();
            }

            let (page1, total) = super::find_all_paginated(&pool, 1, 2, None).await.unwrap();
            assert_eq!(total, 5);
            assert_eq!(page1.len(), 2);

            let (page3, _) = super::find_all_paginated(&pool, 3, 2, None).await.unwrap();
            assert_eq!(page3.len(), 1);
        }
    }
}
