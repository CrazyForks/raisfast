//! Deterministic seed execution (app-bundle.md §6).
//!
//! Idempotency is the lifeline: seeds are upserts with stable identifiers,
//! never blind inserts. Option seeds use `INSERT ... DO NOTHING` semantics so
//! user-edited defaults are never overwritten by an upgrade re-run; CT rows
//! upsert by the explicit `seed_key` column; roles are namespaced
//! (`{app_id}:{name}`) and created only when missing.

use serde_json::Value;

use crate::apps::model::{LogStep, UndoAction};
use crate::content_type::repository::{BindValue, field_bind};
use crate::content_type::schema::ContentTypeSchema;
use crate::db::DbDriver;
use crate::errors::app_error::{AppError, AppResult};

/// What the seeder did (feeds the compensation log).
#[derive(Debug, Default)]
pub struct SeedOutcome {
    pub option_keys: Vec<String>,
    pub seed_role_ids: Vec<i64>,
    /// (table, seed_keys) per CT seed file.
    pub ct_rows: Vec<(String, Vec<String>)>,
}

/// Async progress sink — persists the compensation log after every append.
#[allow(async_fn_in_trait)]
pub trait StepSink {
    async fn commit(&mut self, step: LogStep) -> AppResult<()>;
}

/// Run option / role / CT-row seeds. Each category commits one log step with
/// its undo descriptor generated right here (§4.2 discipline).
pub async fn run<S: StepSink>(
    pool: &crate::db::Pool,
    app_id: &str,
    option_seeds: &[Value],
    role_seeds: &[Value],
    ct_schemas: &[ContentTypeSchema],
    ct_seeds: &[(String, Vec<Value>)],
    progress: &mut S,
) -> AppResult<SeedOutcome> {
    let mut outcome = SeedOutcome::default();

    seed_options(pool, option_seeds, progress, &mut outcome).await?;
    seed_roles(pool, app_id, role_seeds, progress, &mut outcome).await?;
    seed_ct_rows(pool, ct_schemas, ct_seeds, progress, &mut outcome).await?;
    Ok(outcome)
}

/// Option seeds: `app.{app_id}.*` rows via insert-ignore (§6.2 — DO NOTHING,
/// user-edited values survive upgrade re-runs).
async fn seed_options<S: StepSink>(
    pool: &crate::db::Pool,
    seeds: &[Value],
    progress: &mut S,
    outcome: &mut SeedOutcome,
) -> AppResult<()> {
    if seeds.is_empty() {
        return Ok(());
    }
    let now = crate::utils::tz::now_utc();
    for seed in seeds {
        let key = seed
            .get("key")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::BadRequest("option seed missing key".into()))?;
        let value = seed.get("value").cloned().unwrap_or(Value::Null);
        let type_ = seed.get("type").and_then(Value::as_str).unwrap_or("text");
        let label = seed.get("label").and_then(Value::as_str).unwrap_or(key);
        let group = seed.get("group").and_then(Value::as_str).unwrap_or("apps");
        let is_public = seed
            .get("is_public")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let sort_order = seed
            .get("sort_order")
            .and_then(Value::as_i64)
            .unwrap_or(100);

        let sql = crate::db::Driver::insert_ignore_sql(
            "options",
            "id, tenant_id, option_key, value, type, group_name, label, description, \
             validation, is_public, autoload, sort_order, updated_at",
            &(1..=13)
                .map(crate::db::Driver::ph)
                .collect::<Vec<_>>()
                .join(", "),
        );
        let q = sqlx::query(crate::db::safe_sql(&sql))
            .bind(crate::utils::id::new_id())
            .bind(crate::constants::DEFAULT_TENANT)
            .bind(key)
            .bind(value)
            .bind(type_)
            .bind(group)
            .bind(label)
            .bind(seed.get("description").and_then(Value::as_str))
            .bind(seed.get("validation"))
            .bind(is_public)
            .bind(false)
            .bind(sort_order)
            .bind(now);
        let _ = q.execute(pool).await?; // ignore-mode insert: conflicts are the point
        outcome.option_keys.push(key.to_string());
    }
    progress
        .commit(LogStep {
            seq: 0,
            step: "seeds/options".to_string(),
            detail: format!(
                "{} option(s) (insert-if-missing)",
                outcome.option_keys.len()
            ),
            undo: UndoAction::DeleteOptions {
                keys: outcome.option_keys.clone(),
            },
            done: true,
        })
        .await?;
    Ok(())
}

/// Role seeds: namespaced `{app_id}:{name}`, created only when missing;
/// declared permissions attach as `{app_id}:{permission}` subjects.
async fn seed_roles<S: StepSink>(
    pool: &crate::db::Pool,
    app_id: &str,
    seeds: &[Value],
    progress: &mut S,
    outcome: &mut SeedOutcome,
) -> AppResult<()> {
    if seeds.is_empty() {
        return Ok(());
    }
    for seed in seeds {
        let name = seed
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::BadRequest("role seed missing name".into()))?;
        let full_name = format!("{app_id}:{name}");
        let role_id = match crate::models::rbac::find_role_id_by_name(pool, &full_name).await? {
            Some(id) => id,
            None => {
                crate::models::rbac::create_role(
                    pool,
                    &full_name,
                    seed.get("description").and_then(Value::as_str),
                )
                .await?
                .id
                .0
            }
        };
        if let Some(perms) = seed.get("permissions").and_then(Value::as_array) {
            for perm in perms {
                let perm = perm.as_str().unwrap_or_default();
                if perm.is_empty() {
                    continue;
                }
                // Idempotent: skip when this (role, subject) pair exists.
                let sql = format!(
                    "SELECT id FROM permissions WHERE role_id = {} AND subject = {}",
                    crate::db::Driver::ph(1),
                    crate::db::Driver::ph(2)
                );
                let exists: Option<(i64,)> = sqlx::query_as(crate::db::safe_sql(&sql))
                    .bind(role_id)
                    .bind(format!("{app_id}:{perm}"))
                    .fetch_optional(pool)
                    .await?;
                if exists.is_some() {
                    continue;
                }
                crate::models::rbac::insert_permission(
                    pool,
                    &crate::commands::CreatePermissionCmd {
                        role_id: crate::types::snowflake_id::SnowflakeId(role_id),
                        action: "allow".into(),
                        subject: format!("{app_id}:{perm}"),
                        fields: None,
                        conditions: None,
                    },
                )
                .await?;
            }
        }
        outcome.seed_role_ids.push(role_id);
    }
    progress
        .commit(LogStep {
            seq: 0,
            step: "seeds/roles".to_string(),
            detail: format!("{} role(s)", outcome.seed_role_ids.len()),
            undo: UndoAction::Noop, // per-role undo steps appended below
            done: true,
        })
        .await?;
    for role_id in outcome.seed_role_ids.clone() {
        progress
            .commit(LogStep {
                seq: 0,
                step: "seeds/roles".to_string(),
                detail: format!("role row {role_id}"),
                undo: UndoAction::DropSeedRole {
                    role_id: role_id.to_string(),
                },
                done: true,
            })
            .await?;
    }
    Ok(())
}

/// CT data seeds: upsert by `seed_key` (§6.1 — explicit string identity,
/// primary keys stay Snowflake).
async fn seed_ct_rows<S: StepSink>(
    pool: &crate::db::Pool,
    ct_schemas: &[ContentTypeSchema],
    ct_seeds: &[(String, Vec<Value>)],
    progress: &mut S,
    outcome: &mut SeedOutcome,
) -> AppResult<()> {
    for (table, rows) in ct_seeds {
        let schema = ct_schemas
            .iter()
            .find(|s| s.table == *table)
            .ok_or_else(|| AppError::BadRequest(format!("seed targets unknown table '{table}'")))?;
        let mut seed_keys = Vec::new();
        for row in rows {
            let obj = row
                .as_object()
                .ok_or_else(|| AppError::BadRequest("seed row must be an object".into()))?;
            let seed_key = obj
                .get("seed_key")
                .and_then(Value::as_str)
                .ok_or_else(|| AppError::BadRequest("seed row missing seed_key".into()))?
                .to_string();
            upsert_seed_row(pool, schema, &seed_key, obj).await?;
            seed_keys.push(seed_key);
        }
        outcome.ct_rows.push((table.clone(), seed_keys.clone()));
        progress
            .commit(LogStep {
                seq: 0,
                step: "seeds/data".to_string(),
                detail: format!("{table}: {} row(s)", seed_keys.len()),
                undo: UndoAction::DeleteSeedRows {
                    table: table.clone(),
                    seed_keys,
                },
                done: true,
            })
            .await?;
    }
    Ok(())
}

/// Upsert one seed row: select by `seed_key`, update when present, insert
/// otherwise. Values are bound with the CT field-type-aware encoder.
async fn upsert_seed_row(
    pool: &crate::db::Pool,
    schema: &ContentTypeSchema,
    seed_key: &str,
    obj: &serde_json::Map<String, Value>,
) -> AppResult<()> {
    let fields: Vec<(&crate::content_type::schema::FieldSchema, &Value)> = schema
        .fields
        .iter()
        .filter_map(|f| obj.get(&f.name).map(|v| (f, v)))
        .collect();
    // Reject unknown keys (typos in seeds must fail loudly, not silently skip).
    for key in obj.keys() {
        if key == "seed_key" {
            continue;
        }
        if !schema.fields.iter().any(|f| &f.name == key) {
            return Err(AppError::BadRequest(format!(
                "seed row for '{}' references unknown field '{key}'",
                schema.table
            )));
        }
    }

    let select = format!(
        "SELECT id FROM {} WHERE seed_key = {}",
        schema.table,
        crate::db::Driver::ph(1)
    );
    let existing: Option<(i64,)> = sqlx::query_as(crate::db::safe_sql(&select))
        .bind(seed_key)
        .fetch_optional(pool)
        .await?;

    match existing {
        Some((row_id,)) => {
            let mut sets = vec![format!("seed_key = {}", crate::db::Driver::ph(1))];
            let mut binds: Vec<BindValue> = vec![BindValue::Text(seed_key.to_string())];
            for (i, (field, value)) in fields.iter().enumerate() {
                sets.push(format!("{} = {}", field.name, crate::db::Driver::ph(2 + i)));
                binds.push(field_bind(&field.field_type, value));
            }
            sets.push(format!(
                "updated_at = {}",
                crate::db::Driver::ph(2 + fields.len())
            ));
            binds.push(BindValue::Timestamp(crate::utils::tz::now_utc()));
            let sql = format!(
                "UPDATE {} SET {} WHERE id = {}",
                schema.table,
                sets.join(", "),
                crate::db::Driver::ph(3 + fields.len())
            );
            let mut q = sqlx::query(crate::db::safe_sql(&sql));
            for b in binds {
                q = b.bind(q);
            }
            q.bind(row_id).execute(pool).await?;
        }
        None => {
            let mut cols = vec!["id".to_string(), "seed_key".to_string()];
            let mut binds: Vec<BindValue> = vec![
                BindValue::Int(crate::utils::id::new_id()),
                BindValue::Text(seed_key.to_string()),
            ];
            for (field, value) in &fields {
                cols.push(field.name.clone());
                binds.push(field_bind(&field.field_type, value));
            }
            let placeholders = (1..=cols.len())
                .map(crate::db::Driver::ph)
                .collect::<Vec<_>>();
            let sql = format!(
                "INSERT INTO {} ({}) VALUES ({})",
                schema.table,
                cols.join(", "),
                placeholders.join(", ")
            );
            let mut q = sqlx::query(crate::db::safe_sql(&sql));
            for b in binds {
                q = b.bind(q);
            }
            q.execute(pool).await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Progress sink that just records steps (no DB persistence).
    struct MemSink(Vec<LogStep>);
    impl StepSink for MemSink {
        async fn commit(&mut self, step: LogStep) -> AppResult<()> {
            self.0.push(step);
            Ok(())
        }
    }

    #[tokio::test]
    async fn option_seeds_do_nothing_on_conflict() {
        let pool = crate::test_pool!();
        let key = format!("app.demo-{}.enabled", crate::utils::id::new_id());

        // First seed writes true.
        let mut sink = MemSink(Vec::new());
        run(
            &pool,
            "demo",
            &[json!({"key": key, "value": true, "type": "boolean"})],
            &[],
            &[],
            &[],
            &mut sink,
        )
        .await
        .expect("seed 1");
        let value: String = sqlx::query_scalar(crate::db::safe_sql(&format!(
            "SELECT {} FROM options WHERE option_key = '{key}'",
            crate::db::Driver::cast_text("value")
        )))
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(value, "true");

        // Re-run with a different value: DO NOTHING keeps the user's value.
        let mut sink2 = MemSink(Vec::new());
        run(
            &pool,
            "demo",
            &[json!({"key": key, "value": false, "type": "boolean"})],
            &[],
            &[],
            &[],
            &mut sink2,
        )
        .await
        .expect("seed 2");
        let value: String = sqlx::query_scalar(crate::db::safe_sql(&format!(
            "SELECT {} FROM options WHERE option_key = '{key}'",
            crate::db::Driver::cast_text("value")
        )))
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(value, "true", "user-edited option survives a re-run");

        // Compensation log carries the undo.
        assert!(matches!(sink.0[0].undo, UndoAction::DeleteOptions { .. }));
    }
}
