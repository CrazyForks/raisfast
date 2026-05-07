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

use crate::db::tenant::{tenant_filter, tenant_filter_aliased};
use crate::errors::app_error::{AppError, AppResult};

/// 评论完整数据库行模型
///
/// 直接映射 `comments` 表的所有字段。
/// `created_by` 非空表示已登录用户，`nickname`/`email` 用于访客评论。
/// `status` 可取 `pending`、`approved`、`rejected`。
#[derive(Debug, Serialize, Deserialize, Clone)]
#[non_exhaustive]
pub struct Comment {
    pub id: String,
    pub tenant_id: Option<String>,
    pub post_id: String,
    pub created_by: Option<String>,
    pub updated_by: Option<String>,
    pub nickname: Option<String>,
    pub email: Option<String>,
    pub content: String,
    pub parent_id: Option<String>,
    pub status: String,
    pub created_at: String,
    pub updated_at: Option<String>,
}

crate::impl_from_row_opt_tenant!(Comment {
    required { id, post_id, content, status, created_at }
    optional { created_by, updated_by, nickname, email, parent_id, updated_at }
});

/// 评论 API 响应模型（树形结构）
///
/// 在 [`Comment`] 基础上增加 `depth`（嵌套深度）和 `replies`（子评论列表），
/// 形成递归的树形结构。
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, Clone)]
#[non_exhaustive]
pub struct CommentResponse {
    pub id: String,
    pub post_id: String,
    pub created_by: Option<String>,
    pub nickname: Option<String>,
    pub content: String,
    pub parent_id: Option<String>,
    pub depth: i32,
    pub replies: Vec<CommentResponse>,
    pub created_at: String,
}

/// 根据评论 ID 查找评论
///
/// 返回 `Ok(Some(comment))` 或 `Ok(None)`（未找到时）。
pub async fn find_by_id(
    pool: &crate::db::Pool,
    id: &str,
    tenant_id: Option<&str>,
) -> AppResult<Option<Comment>> {
    let sql_str = format!(
        "SELECT * FROM comments WHERE id = ?{}",
        tenant_filter(tenant_id)
    );
    let sql = crate::db::dialect::translate(&sql_str);
    let mut q = sqlx::query_as::<_, Comment>(&sql).bind(id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    let comment = q.fetch_optional(pool).await?;
    Ok(comment)
}

/// 创建新评论
///
/// 自动生成 UUID v7 作为主键，初始状态为 `pending`。
/// 创建完成后重新查询并返回完整评论记录。
pub async fn create(
    pool: &crate::db::Pool,
    cmd: &crate::commands::CreateCommentCmd,
    tenant_id: Option<&str>,
) -> AppResult<Comment> {
    let (id, now) = crate::utils::id::new_id_and_timestamp();

    match tenant_id {
        Some(tid) => {
            let sql = crate::db::dialect::translate(
                "INSERT INTO comments (id, tenant_id, post_id, created_by, updated_by, nickname, email, content, parent_id, status, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 'pending', ?, ?)",
            );
            sqlx::query(&sql)
                .bind(&id)
                .bind(tid)
                .bind(&cmd.post_id)
                .bind(&cmd.created_by)
                .bind(&cmd.created_by)
                .bind(&cmd.nickname)
                .bind(&cmd.email)
                .bind(&cmd.content)
                .bind(&cmd.parent_id)
                .bind(&now)
                .bind(&now)
                .execute(pool)
                .await?;
        }
        None => {
            let sql = crate::db::dialect::translate(
                "INSERT INTO comments (id, post_id, created_by, updated_by, nickname, email, content, parent_id, status, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'pending', ?, ?)",
            );
            sqlx::query(&sql)
                .bind(&id)
                .bind(&cmd.post_id)
                .bind(&cmd.created_by)
                .bind(&cmd.created_by)
                .bind(&cmd.nickname)
                .bind(&cmd.email)
                .bind(&cmd.content)
                .bind(&cmd.parent_id)
                .bind(&now)
                .bind(&now)
                .execute(pool)
                .await?;
        }
    }

    find_by_id(pool, &id, tenant_id)
        .await?
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("failed to fetch created comment")))
}

/// 查询指定文章下已审核通过的评论
///
/// 按 `created_at` 升序排列。
pub async fn find_approved_by_post(
    pool: &crate::db::Pool,
    post_id: &str,
    tenant_id: Option<&str>,
) -> AppResult<Vec<Comment>> {
    let sql_str = format!(
        "SELECT * FROM comments WHERE post_id = ? AND status = 'approved'{} ORDER BY created_at ASC",
        tenant_filter(tenant_id)
    );
    let sql = crate::db::dialect::translate(&sql_str);
    let mut q = sqlx::query_as::<_, Comment>(&sql).bind(post_id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    let comments = q.fetch_all(pool).await?;
    Ok(comments)
}

/// 分页查询指定文章下已审核通过的评论
///
/// 按 `created_at` 升序排列。返回评论列表和总记录数。
pub async fn find_approved_by_post_paginated(
    pool: &crate::db::Pool,
    post_id: &str,
    page: i64,
    page_size: i64,
    tenant_id: Option<&str>,
) -> AppResult<(Vec<Comment>, i64)> {
    let offset = (page - 1) * page_size;
    let sql_str = format!(
        "SELECT * FROM comments WHERE post_id = ? AND status = 'approved'{} ORDER BY created_at ASC LIMIT ? OFFSET ?",
        tenant_filter(tenant_id)
    );
    let sql = crate::db::dialect::translate(&sql_str);
    let mut q = sqlx::query_as::<_, Comment>(&sql).bind(post_id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    q = q.bind(page_size).bind(offset);
    let comments = q.fetch_all(pool).await?;

    let sql_str = format!(
        "SELECT COUNT(*) FROM comments WHERE post_id = ? AND status = 'approved'{}",
        tenant_filter(tenant_id)
    );
    let sql = crate::db::dialect::translate(&sql_str);
    let mut q2 = sqlx::query_scalar::<_, i64>(&sql).bind(post_id);
    if let Some(tid) = tenant_id {
        q2 = q2.bind(tid);
    }
    let total: i64 = q2.fetch_one(pool).await?;

    Ok((comments, total))
}

/// 查询指定文章下的所有评论（含未审核）
///
/// 按 `created_at` 升序排列。仅管理员使用。
pub async fn find_all_by_post(
    pool: &crate::db::Pool,
    post_id: &str,
    tenant_id: Option<&str>,
) -> AppResult<Vec<Comment>> {
    let sql_str = format!(
        "SELECT * FROM comments WHERE post_id = ?{} ORDER BY created_at ASC",
        tenant_filter(tenant_id)
    );
    let sql = crate::db::dialect::translate(&sql_str);
    let mut q = sqlx::query_as::<_, Comment>(&sql).bind(post_id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    let comments = q.fetch_all(pool).await?;
    Ok(comments)
}

/// 分页查询全局所有评论（管理员），关联文章标题。
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, FromRow, Serialize, Clone)]
pub struct AdminCommentRow {
    pub id: String,
    pub post_id: String,
    pub post_title: String,
    pub created_by: Option<String>,
    pub nickname: Option<String>,
    pub email: Option<String>,
    pub content: String,
    pub parent_id: Option<String>,
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
    let sql_str = format!(
        "SELECT c.id, c.post_id, p.title AS post_title, c.created_by, c.nickname, c.email, c.content, c.parent_id, c.status, c.created_at FROM comments c JOIN posts p ON c.post_id = p.id WHERE 1=1{} ORDER BY c.created_at DESC LIMIT ? OFFSET ?",
        tenant_filter_aliased("c", tenant_id)
    );
    let sql = crate::db::dialect::translate(&sql_str);
    let mut q = sqlx::query_as::<_, AdminCommentRow>(&sql);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    q = q.bind(page_size).bind(offset);
    let rows = q.fetch_all(pool).await?;

    let sql2 = format!(
        "SELECT COUNT(*) FROM comments WHERE 1=1{}",
        tenant_filter(tenant_id)
    );
    let sql2 = crate::db::dialect::translate(&sql2);
    let mut q2 = sqlx::query_scalar::<_, i64>(&sql2);
    if let Some(tid) = tenant_id {
        q2 = q2.bind(tid);
    }
    let total: i64 = q2.fetch_one(pool).await?;

    Ok((rows, total))
}

/// 更新评论审核状态
///
/// 若评论不存在则返回 [`AppError::NotFound`]。
pub async fn update_status(
    pool: &crate::db::Pool,
    id: &str,
    status: &str,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    let (_, now) = crate::utils::id::new_id_and_timestamp();
    let sql = format!(
        "UPDATE comments SET status = ?, updated_at = ? WHERE id = ?{}",
        tenant_filter(tenant_id)
    );
    let sql = crate::db::dialect::translate(&sql);
    let mut q = sqlx::query(&sql).bind(status).bind(&now).bind(id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    let result = q.execute(pool).await?;

    AppError::expect_affected(&result, "comment")
}

/// 删除评论
///
/// 若评论不存在则返回 [`AppError::NotFound`]。
pub async fn delete(pool: &crate::db::Pool, id: &str, tenant_id: Option<&str>) -> AppResult<()> {
    let sql = format!(
        "DELETE FROM comments WHERE id = ?{}",
        tenant_filter(tenant_id)
    );
    let sql = crate::db::dialect::translate(&sql);
    let mut q = sqlx::query(&sql).bind(id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    let result = q.execute(pool).await?;

    AppError::expect_affected(&result, "comment")
}

/// 计算评论的嵌套深度
///
/// 通过沿 `parent_id` 链向上遍历来计算当前评论的嵌套层级。
/// 使用 `visited` 集合防止循环引用，深度超过 10 时自动中断。
fn get_depth(comments: &[Comment], comment: &Comment) -> i32 {
    let mut depth = 0;
    let mut current_parent = comment.parent_id.clone();
    let mut visited = std::collections::HashSet::new();
    while let Some(pid) = current_parent {
        if visited.contains(&pid) || depth > 10 {
            break;
        }
        visited.insert(pid.clone());
        depth += 1;
        current_parent = comments
            .iter()
            .find(|c| c.id == pid)
            .and_then(|c| c.parent_id.clone());
    }
    depth
}

/// 将扁平评论列表构建为嵌套树结构
///
/// 使用 `HashMap` 按 `parent_id` 分组，然后递归构建子评论树。
/// 顶层评论的 `parent_id` 为 `None`（用空字符串作为 key）。
#[must_use]
pub fn build_tree(comments: &[Comment]) -> Vec<CommentResponse> {
    let map: std::collections::HashMap<String, Vec<Comment>> =
        comments
            .iter()
            .fold(std::collections::HashMap::new(), |mut acc, c| {
                let key = c.parent_id.clone().unwrap_or_default();
                acc.entry(key).or_default().push(c.clone());
                acc
            });

    fn build(
        parent_id: &str,
        map: &std::collections::HashMap<String, Vec<Comment>>,
        comments: &[Comment],
    ) -> Vec<CommentResponse> {
        map.get(parent_id)
            .map(|children| {
                children
                    .iter()
                    .map(|c| {
                        let depth = get_depth(comments, c);
                        let replies = build(&c.id, map, comments);
                        CommentResponse {
                            id: c.id.clone(),
                            post_id: c.post_id.clone(),
                            created_by: c.created_by.clone(),
                            nickname: c.nickname.clone(),
                            content: c.content.clone(),
                            parent_id: c.parent_id.clone(),
                            depth,
                            replies,
                            created_at: c.created_at.clone(),
                        }
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    build("", &map, comments)
}

/// 评论嵌套最大深度限制
const MAX_DEPTH: i32 = 3;

/// 验证评论嵌套深度不超过最大限制
///
/// 检查父评论的当前深度，若已达到或超过 [`MAX_DEPTH`]（3 层），
/// 则返回 [`AppError::BadRequest`]。
pub fn validate_depth(comments: &[Comment], parent_id: &str) -> AppResult<()> {
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

    fn make_comment(id: &str, post_id: &str, parent_id: Option<&str>) -> Comment {
        Comment {
            id: id.to_string(),
            tenant_id: Some(crate::constants::DEFAULT_TENANT.to_string()),
            post_id: post_id.to_string(),
            created_by: None,
            updated_by: None,
            nickname: None,
            email: None,
            content: "test".to_string(),
            parent_id: parent_id.map(|s| s.to_string()),
            status: "approved".to_string(),
            created_at: "2025-01-01T00:00:00Z".to_string(),
            updated_at: None,
        }
    }

    #[test]
    fn build_tree_flat_comments() {
        let comments = vec![make_comment("1", "p1", None), make_comment("2", "p1", None)];
        let tree = build_tree(&comments);
        assert_eq!(tree.len(), 2);
        assert!(tree[0].replies.is_empty());
        assert!(tree[1].replies.is_empty());
    }

    #[test]
    fn build_tree_nested() {
        let comments = vec![
            make_comment("1", "p1", None),
            make_comment("2", "p1", Some("1")),
            make_comment("3", "p1", Some("2")),
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
            make_comment("1", "p1", None),
            make_comment("2", "p1", Some("1")),
            make_comment("3", "p1", Some("2")),
        ];
        let tree = build_tree(&comments);
        assert_eq!(tree[0].depth, 0);
        assert_eq!(tree[0].replies[0].depth, 1);
        assert_eq!(tree[0].replies[0].replies[0].depth, 2);
    }

    #[test]
    fn validate_depth_ok_within_limit() {
        let comments = vec![
            make_comment("1", "p1", None),
            make_comment("2", "p1", Some("1")),
        ];
        assert!(validate_depth(&comments, "2").is_ok());
    }

    #[test]
    fn validate_depth_fails_at_max() {
        let comments = vec![
            make_comment("1", "p1", None),
            make_comment("2", "p1", Some("1")),
            make_comment("3", "p1", Some("2")),
            make_comment("4", "p1", Some("3")),
        ];
        assert!(validate_depth(&comments, "4").is_err());
    }

    #[test]
    fn validate_depth_missing_parent() {
        let comments = vec![make_comment("1", "p1", None)];
        assert!(validate_depth(&comments, "nonexistent").is_err());
    }
}
