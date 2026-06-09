use crate::errors::app_error::AppResult;
use crate::types::snowflake_id::SnowflakeId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct Tagging {
    pub id: SnowflakeId,
    pub tag_id: SnowflakeId,
    pub taggable_type: String,
    pub taggable_id: SnowflakeId,
    pub tenant_id: Option<String>,
}

pub async fn sync_tags_tx(
    tx: &mut crate::db::Transaction<'_>,
    taggable_type: &str,
    taggable_id: SnowflakeId,
    tag_ids: &[i64],
    tenant_id: Option<&str>,
) -> AppResult<()> {
    raisfast_derive::crud_delete!(
        &mut **tx, "taggings",
        where: AND(("taggable_type", taggable_type), ("taggable_id", taggable_id)),
        tenant: tenant_id
    )?;

    for tag_id in tag_ids {
        let id = crate::utils::id::new_snowflake_id();
        raisfast_derive::crud_insert!(
            &mut **tx, "taggings",
            [
                "id" => id,
                "tag_id" => *tag_id,
                "taggable_type" => taggable_type,
                "taggable_id" => taggable_id
            ],
            tenant: tenant_id
        )?;
    }

    Ok(())
}

pub async fn count_by_tag_id(pool: &crate::db::Pool, tag_id: i64) -> AppResult<i64> {
    raisfast_derive::crud_count!(pool, "taggings", where: ("tag_id", tag_id)).map_err(Into::into)
}

pub async fn get_tags_for(
    pool: &crate::db::Pool,
    taggable_type: &str,
    taggable_id: SnowflakeId,
) -> AppResult<Vec<crate::models::post::TagBrief>> {
    let rows: Vec<crate::models::post::TagRow> = raisfast_derive::crud_join!(
        pool, crate::models::post::TagRow,
        select: ["t.id", "t.name", "t.slug"],
        from: "tags t",
        joins: [INNER "taggings tg" ON "t.id = tg.tag_id"],
        where: AND(("tg.taggable_type", taggable_type), ("tg.taggable_id", taggable_id)),
        method: fetch_all
    )?;

    Ok(rows
        .into_iter()
        .map(|r| crate::models::post::TagBrief {
            id: r.id.to_string(),
            name: r.name,
            slug: r.slug,
        })
        .collect())
}

pub async fn get_tags_for_posts(
    pool: &crate::db::Pool,
    taggable_type: &str,
    post_ids: &[SnowflakeId],
) -> AppResult<std::collections::HashMap<SnowflakeId, Vec<crate::models::post::TagBrief>>> {
    if post_ids.is_empty() {
        return Ok(std::collections::HashMap::new());
    }

    #[derive(Debug, sqlx::FromRow)]
    struct TagWithPostId {
        taggable_id: SnowflakeId,
        id: SnowflakeId,
        name: String,
        slug: String,
    }

    let rows: Vec<TagWithPostId> = raisfast_derive::crud_join!(
        pool,
        TagWithPostId,
        select: ["tg.taggable_id", "t.id", "t.name", "t.slug"],
        from: "taggings tg",
        joins: [JOIN "tags t" ON "tg.tag_id = t.id"],
        where: AND(("tg.taggable_type", taggable_type), ("tg.taggable_id", IN, post_ids)),
        method: fetch_all
    )?;

    let mut map: std::collections::HashMap<SnowflakeId, Vec<crate::models::post::TagBrief>> =
        std::collections::HashMap::new();
    for row in rows {
        map.entry(row.taggable_id)
            .or_default()
            .push(crate::models::post::TagBrief {
                id: row.id.to_string(),
                name: row.name,
                slug: row.slug,
            });
    }
    Ok(map)
}
