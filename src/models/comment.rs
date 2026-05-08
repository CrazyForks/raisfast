//! 评论模型与数据库查询
//!
//! 定义评论（Comment）相关的数据结构，包括完整行模型、支持嵌套树结构的响应模型、
//! 请求验证结构体，以及对 `comments` 表的增删改查操作。
//!
//! 评论支持多级嵌套回复，通过 `parent_id` 构建父子关系。
//! 树结构由 [`build_tree`] 函数从扁平列表转换而来，
//! 嵌套深度由 [`validate_depth`] 限制为最多 3 层。

use serde::{Deserialize, Serialize};
use sqlx::FromRow;
#[cfg(feature = "export-types")]
use ts_rs::TS;

use crate::db::dialect::ph;
use crate::db::tenant::{tenant_filter_aliased_ph, tenant_filter_ph};
use crate::errors::app_error::{AppError, AppResult};

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
    pub status: String,
    pub created_at: String,
    pub updated_at: Option<String>,
}

crate::impl_from_row_opt_tenant!(Comment {
    required { id, document_id, post_id, content, status, created_at }
    optional { created_by, updated_by, nickname, email, parent_id, updated_at }
});

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, Clone)]
#[non_exhaustive]
pub struct CommentResponse {
    pub id: i64,
    pub post_id: i64,
    pub created_by: Option<i64>,
    pub nickname: Option<String>,
    pub content: String,
    pub parent_id: Option<i64>,
    pub depth: i32,
    pub replies: Vec<CommentResponse>,
    pub created_at: String,
}

pub async fn find_by_id(
    pool: &crate::db::Pool,
    id: i64,
    tenant_id: Option<&str>,
) -> AppResult<Option<Comment>> {
    let filter = tenant_filter_ph(tenant_id, 2);
    let sql = format!("SELECT * FROM comments WHERE id = {}{filter}", ph(1));
    let mut q = sqlx::query_as::<_, Comment>(&sql).bind(id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    let comment = q.fetch_optional(pool).await?;
    Ok(comment)
}

pub async fn find_by_document_id(
    pool: &crate::db::Pool,
    document_id: &str,
    tenant_id: Option<&str>,
) -> AppResult<Option<Comment>> {
    let filter = tenant_filter_ph(tenant_id, 2);
    let sql = format!(
        "SELECT * FROM comments WHERE document_id = {}{filter}",
        ph(1)
    );
    let mut q = sqlx::query_as::<_, Comment>(&sql).bind(document_id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    let comment = q.fetch_optional(pool).await?;
    Ok(comment)
}

pub async fn create(
    pool: &crate::db::Pool,
    cmd: &crate::commands::CreateCommentCmd,
    tenant_id: Option<&str>,
) -> AppResult<Comment> {
    let (document_id, now) = crate::utils::id::new_document_id_and_timestamp();

    match tenant_id {
        Some(tid) => {
            let vals1 = (1..=9).map(ph).collect::<Vec<_>>().join(", ");
            let vals2 = (10..=11).map(ph).collect::<Vec<_>>().join(", ");
            let sql = format!(
                "INSERT INTO comments (document_id, tenant_id, post_id, created_by, updated_by, nickname, email, content, parent_id, status, created_at, updated_at) VALUES ({vals1}, 'pending', {vals2})"
            );
            sqlx::query(&sql)
                .bind(&document_id)
                .bind(tid)
                .bind(cmd.post_id)
                .bind(cmd.created_by)
                .bind(cmd.created_by)
                .bind(&cmd.nickname)
                .bind(&cmd.email)
                .bind(&cmd.content)
                .bind(cmd.parent_id)
                .bind(&now)
                .bind(&now)
                .execute(pool)
                .await?;
        }
        None => {
            let vals1 = (1..=8).map(ph).collect::<Vec<_>>().join(", ");
            let vals2 = (9..=10).map(ph).collect::<Vec<_>>().join(", ");
            let sql = format!(
                "INSERT INTO comments (document_id, post_id, created_by, updated_by, nickname, email, content, parent_id, status, created_at, updated_at) VALUES ({vals1}, 'pending', {vals2})"
            );
            sqlx::query(&sql)
                .bind(&document_id)
                .bind(cmd.post_id)
                .bind(cmd.created_by)
                .bind(cmd.created_by)
                .bind(&cmd.nickname)
                .bind(&cmd.email)
                .bind(&cmd.content)
                .bind(cmd.parent_id)
                .bind(&now)
                .bind(&now)
                .execute(pool)
                .await?;
        }
    }

    find_by_document_id(pool, &document_id, tenant_id)
        .await?
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("failed to fetch created comment")))
}

pub async fn find_approved_by_post(
    pool: &crate::db::Pool,
    post_id: i64,
    tenant_id: Option<&str>,
) -> AppResult<Vec<Comment>> {
    let filter = tenant_filter_ph(tenant_id, 2);
    let sql = format!(
        "SELECT * FROM comments WHERE post_id = {} AND status = 'approved'{filter} ORDER BY created_at ASC",
        ph(1)
    );
    let mut q = sqlx::query_as::<_, Comment>(&sql).bind(post_id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    let comments = q.fetch_all(pool).await?;
    Ok(comments)
}

pub async fn find_approved_by_post_paginated(
    pool: &crate::db::Pool,
    post_id: i64,
    page: i64,
    page_size: i64,
    tenant_id: Option<&str>,
) -> AppResult<(Vec<Comment>, i64)> {
    let offset = (page - 1) * page_size;
    let filter = tenant_filter_ph(tenant_id, 2);
    let base = usize::from(tenant_id.is_some());
    let sql = format!(
        "SELECT * FROM comments WHERE post_id = {} AND status = 'approved'{filter} ORDER BY created_at ASC LIMIT {} OFFSET {}",
        ph(1),
        ph(base + 2),
        ph(base + 3)
    );
    let mut q = sqlx::query_as::<_, Comment>(&sql).bind(post_id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    q = q.bind(page_size).bind(offset);
    let comments = q.fetch_all(pool).await?;

    let filter2 = tenant_filter_ph(tenant_id, 2);
    let sql2 = format!(
        "SELECT COUNT(*) FROM comments WHERE post_id = {} AND status = 'approved'{filter2}",
        ph(1)
    );
    let mut q2 = sqlx::query_scalar::<_, i64>(&sql2).bind(post_id);
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
    let mut q = sqlx::query_as::<_, Comment>(&sql).bind(post_id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    let comments = q.fetch_all(pool).await?;
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
    pub status: String,
    pub created_at: String,
}

pub async fn find_all_paginated(
    pool: &crate::db::Pool,
    page: i64,
    page_size: i64,
    tenant_id: Option<&str>,
) -> AppResult<(Vec<AdminCommentRow>, i64)> {
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
    status: &str,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    let (_, now) = crate::utils::id::new_document_id_and_timestamp();
    let filter = tenant_filter_ph(tenant_id, 4);
    let sql = format!(
        "UPDATE comments SET status = {}, updated_at = {} WHERE id = {}{filter}",
        ph(1),
        ph(2),
        ph(3)
    );
    let mut q = sqlx::query(&sql).bind(status).bind(&now).bind(id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    let result = q.execute(pool).await?;

    AppError::expect_affected(&result, "comment")
}

pub async fn delete(pool: &crate::db::Pool, id: i64, tenant_id: Option<&str>) -> AppResult<()> {
    let filter = tenant_filter_ph(tenant_id, 2);
    let sql = format!("DELETE FROM comments WHERE id = {}{filter}", ph(1));
    let mut q = sqlx::query(&sql).bind(id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    let result = q.execute(pool).await?;

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
                            post_id: c.post_id,
                            created_by: c.created_by,
                            nickname: c.nickname.clone(),
                            content: c.content.clone(),
                            parent_id: c.parent_id,
                            depth,
                            replies,
                            created_at: c.created_at.clone(),
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
            status: "approved".to_string(),
            created_at: "2025-01-01T00:00:00Z".to_string(),
            updated_at: None,
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
}
