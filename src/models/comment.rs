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

use crate::errors::app_error::{AppError, AppResult};
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::tz::Timestamp;

define_enum!(
    CommentStatus {
        Pending = "pending",
        Approved = "approved",
        Spam = "spam",
    }
);

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
#[non_exhaustive]
pub struct Comment {
    pub id: SnowflakeId,
    pub tenant_id: Option<String>,
    pub post_id: SnowflakeId,
    pub created_by: Option<SnowflakeId>,
    pub updated_by: Option<SnowflakeId>,
    pub nickname: Option<String>,
    pub email: Option<String>,
    pub content: String,
    pub parent_id: Option<SnowflakeId>,
    pub author_ip: Option<String>,
    pub author_url: Option<String>,
    pub status: CommentStatus,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, Clone)]
#[non_exhaustive]
pub struct CommentResponse {
    pub id: String,
    pub nickname: Option<String>,
    pub content: String,
    pub depth: i32,
    pub replies: Vec<CommentResponse>,
    pub created_at: Timestamp,
    pub post_id: Option<String>,
    pub created_by: Option<String>,
    pub parent_id: Option<String>,
}

pub async fn find_by_id(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    tenant_id: Option<&str>,
) -> AppResult<Option<Comment>> {
    Ok(
        raisfast_derive::crud_find!(pool, "comments", Comment, where: ("id", id), tenant: tenant_id)?,
    )
}

pub async fn create(
    pool: &crate::db::Pool,
    cmd: &crate::commands::CreateCommentCmd,
    tenant_id: Option<&str>,
) -> AppResult<Comment> {
    let (id, now) = (
        crate::utils::id::new_snowflake_id(),
        crate::utils::tz::now_utc(),
    );

    raisfast_derive::crud_insert!(
        pool,
        "comments",
        [
            "id" => id,
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
        tenant: tenant_id
    )?;

    find_by_id(pool, id, tenant_id)
        .await?
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("failed to fetch created comment")))
}

pub async fn find_approved_by_post(
    pool: &crate::db::Pool,
    post_id: SnowflakeId,
    tenant_id: Option<&str>,
) -> AppResult<Vec<Comment>> {
    Ok(
        raisfast_derive::crud_find_all!(pool, "comments", Comment, where: AND(("post_id", post_id), ("status", CommentStatus::Approved)), tenant: tenant_id, order_by: "created_at ASC")?,
    )
}

pub async fn find_approved_by_post_paginated(
    pool: &crate::db::Pool,
    post_id: SnowflakeId,
    page: i64,
    page_size: i64,
    tenant_id: Option<&str>,
) -> AppResult<(Vec<Comment>, i64)> {
    let result = raisfast_derive::crud_query_paged!(
        pool, Comment,
        table: "comments",
        where: AND(("post_id", post_id), ("status", CommentStatus::Approved)),
        order_by: "created_at ASC",
        tenant: tenant_id,
        page: page,
        page_size: page_size
    );
    Ok(result)
}

pub async fn find_all_by_post(
    pool: &crate::db::Pool,
    post_id: SnowflakeId,
    tenant_id: Option<&str>,
) -> AppResult<Vec<Comment>> {
    raisfast_derive::crud_find_all!(pool, "comments", Comment, where: ("post_id", post_id), tenant: tenant_id, order_by: "created_at ASC").map_err(Into::into)
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, Clone)]
pub struct AdminCommentRow {
    pub id: String,
    pub post_id: String,
    pub post_title: String,
    pub nickname: Option<String>,
    pub email: Option<String>,
    pub content: String,
    pub status: CommentStatus,
    pub created_by: Option<String>,
    pub parent_id: Option<String>,
    pub created_at: Timestamp,
}

#[allow(dead_code)]
#[derive(Debug, FromRow)]
struct AdminCommentRowDb {
    id: SnowflakeId,
    post_id: SnowflakeId,
    post_title: String,
    created_by: Option<SnowflakeId>,
    nickname: Option<String>,
    email: Option<String>,
    content: String,
    parent_id: Option<SnowflakeId>,
    status: CommentStatus,
    created_at: Timestamp,
}

impl From<AdminCommentRowDb> for AdminCommentRow {
    fn from(r: AdminCommentRowDb) -> Self {
        Self {
            id: r.id.to_string(),
            post_id: r.post_id.to_string(),
            post_title: r.post_title,
            created_by: r.created_by.map(|v| v.to_string()),
            nickname: r.nickname,
            email: r.email,
            content: r.content,
            parent_id: r.parent_id.map(|v| v.to_string()),
            status: r.status,
            created_at: r.created_at,
        }
    }
}

pub async fn find_all_paginated(
    pool: &crate::db::Pool,
    page: i64,
    page_size: i64,
    tenant_id: Option<&str>,
) -> AppResult<(Vec<AdminCommentRow>, i64)> {
    let (rows, total) = raisfast_derive::crud_join_paged!(
        pool, AdminCommentRowDb,
        select: ["c.id", "c.post_id", "p.title AS post_title", "c.created_by", "c.nickname", "c.email", "c.content", "c.parent_id", "c.status", "c.created_at"],
        from: "comments c",
        joins: [INNER "posts p" ON "c.post_id = p.id"],
        tenant_alias: "c",
        tenant: tenant_id,
        order_by: "c.created_at DESC",
        page: page,
        page_size: page_size
    );
    Ok((rows.into_iter().map(AdminCommentRow::from).collect(), total))
}

pub async fn update_status(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    status: CommentStatus,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    let now = crate::utils::tz::now_utc();
    let result = raisfast_derive::crud_update!(pool, "comments",
        bind: ["status" => status, "updated_at" => &now],
        where: ("id", id),
        tenant: tenant_id
    )?;

    AppError::expect_affected(&result, "comment")
}

pub async fn delete(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    let result =
        raisfast_derive::crud_delete!(pool, "comments", where: ("id", id), tenant: tenant_id)?;
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
    let root_key = SnowflakeId(0);
    let map: std::collections::HashMap<SnowflakeId, Vec<Comment>> =
        comments
            .iter()
            .fold(std::collections::HashMap::new(), |mut acc, c| {
                let key = c.parent_id.unwrap_or(root_key);
                acc.entry(key).or_default().push(c.clone());
                acc
            });

    fn build(
        parent_id: SnowflakeId,
        map: &std::collections::HashMap<SnowflakeId, Vec<Comment>>,
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
                            id: c.id.to_string(),
                            nickname: c.nickname.clone(),
                            content: c.content.clone(),
                            depth,
                            replies,
                            created_at: c.created_at,
                            post_id: None,
                            created_by: None,
                            parent_id: None,
                        }
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    build(root_key, &map, comments)
}

const MAX_DEPTH: i32 = 3;

pub fn validate_depth(comments: &[Comment], parent_id: SnowflakeId) -> AppResult<()> {
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
    use crate::types::snowflake_id::SnowflakeId;

    fn make_comment(id: i64, post_id: i64, parent_id: Option<i64>) -> Comment {
        Comment {
            id: SnowflakeId(id),
            tenant_id: Some(crate::constants::DEFAULT_TENANT.to_string()),
            post_id: SnowflakeId(post_id),
            created_by: None,
            updated_by: None,
            nickname: None,
            email: None,
            content: "test".to_string(),
            parent_id: parent_id.map(SnowflakeId),
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
        assert_eq!(tree[0].id, "1");
        assert_eq!(tree[0].replies.len(), 1);
        assert_eq!(tree[0].replies[0].id, "2");
        assert_eq!(tree[0].replies[0].replies.len(), 1);
        assert_eq!(tree[0].replies[0].replies[0].id, "3");
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
        assert!(validate_depth(&comments, SnowflakeId(2)).is_ok());
    }

    #[test]
    fn validate_depth_fails_at_max() {
        let comments = vec![
            make_comment(1, 10, None),
            make_comment(2, 10, Some(1)),
            make_comment(3, 10, Some(2)),
            make_comment(4, 10, Some(3)),
        ];
        assert!(validate_depth(&comments, SnowflakeId(4)).is_err());
    }

    #[test]
    fn validate_depth_missing_parent() {
        let comments = vec![make_comment(1, 10, None)];
        assert!(validate_depth(&comments, SnowflakeId(999)).is_err());
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
                    role: None,
                },
                None,
            )
            .await
            .unwrap();
            *user.id
        }

        async fn insert_post(pool: &crate::db::Pool, user_id: i64) -> i64 {
            let post_id = crate::utils::id::new_id();
            let slug = format!("slug-{post_id}");
            sqlx::query(
                "INSERT INTO posts (id, title, slug, content, status, created_by, updated_by) VALUES (?, 'Test', ?, 'content', 'published', ?, ?)",
            )
            .bind(post_id)
            .bind(&slug)
            .bind(user_id)
            .bind(user_id)
            .execute(pool)
            .await
            .unwrap();
            post_id
        }

        fn make_cmd(post_id: i64) -> CreateCommentCmd {
            CreateCommentCmd {
                post_id: SnowflakeId(post_id),
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
            assert_eq!(c.post_id, SnowflakeId(pid));
            assert_eq!(c.content, "hello");
            let found = super::super::find_by_id(&pool, c.id, None)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(found.id, c.id);
        }

        #[tokio::test]
        async fn find_by_id_not_found() {
            let pool = setup_pool().await;
            let result = super::super::find_by_id(&pool, SnowflakeId(99999), None)
                .await
                .unwrap();
            assert!(result.is_none());
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

            let approved = super::super::find_approved_by_post(&pool, SnowflakeId(pid), None)
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

            let (page1, total) =
                super::super::find_approved_by_post_paginated(&pool, SnowflakeId(pid), 1, 2, None)
                    .await
                    .unwrap();
            assert_eq!(total, 5);
            assert_eq!(page1.len(), 2);

            let (page3, _) =
                super::super::find_approved_by_post_paginated(&pool, SnowflakeId(pid), 3, 2, None)
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
            let found = super::super::find_by_id(&pool, c.id, None)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(found.status, CommentStatus::Approved);
        }

        #[tokio::test]
        async fn delete_removes_comment() {
            let pool = setup_pool().await;
            let uid = insert_user(&pool).await;
            let pid = insert_post(&pool, uid).await;
            let c = create(&pool, &make_cmd(pid), None).await.unwrap();
            super::super::delete(&pool, c.id, None).await.unwrap();
            let found = super::super::find_by_id(&pool, c.id, None).await.unwrap();
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

            let (page1, total) = super::super::find_all_paginated(&pool, 1, 2, None)
                .await
                .unwrap();
            assert_eq!(total, 5);
            assert_eq!(page1.len(), 2);

            let (page3, _) = super::super::find_all_paginated(&pool, 3, 2, None)
                .await
                .unwrap();
            assert_eq!(page3.len(), 1);
        }
    }
}
