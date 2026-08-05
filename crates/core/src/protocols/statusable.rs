//! statusable Protocol — configurable status field
//!
//! Supports 2 storage modes:
//! - String mode (default): DB stores `"draft"` / `"published"`
//! - Numeric mapping mode: DB stores `1` / `10` / `99`, API interacts with strings
//!
//! Query filtering is not handled by the protocol but by the API rule engine (`[api.list] filter = 'status = "published"'`).

use serde_json::{Value, json};

use crate::constants::COL_STATUS;
use crate::db::sql_type::{ColumnDef, SqlType};
use crate::protocols::{HookCtx, Protocol, ProtocolDeclaration, StatusMode};
use async_trait::async_trait;

fn declaration_from_schema(
    schema: Option<&crate::content_type::schema::ContentTypeSchema>,
) -> Option<&ProtocolDeclaration> {
    schema.and_then(|s| s.cached_declaration.as_ref())
}

fn to_db_value(label: &str, decl: Option<&ProtocolDeclaration>) -> Value {
    if let Some(d) = decl
        && matches!(d.status_mode, StatusMode::Numeric)
        && let Some(map) = &d.status_map
        && let Some((_, num)) = map.iter().find(|(l, _)| l == label)
    {
        return json!(*num);
    }
    json!(label)
}

fn validate_status(v: &Value, decl: Option<&ProtocolDeclaration>) -> Result<(), anyhow::Error> {
    let Some(d) = decl else {
        return Ok(());
    };
    let Some(values) = &d.status_values else {
        return Ok(());
    };

    let label = match d.status_mode {
        StatusMode::Numeric => {
            let num = v.as_i64().unwrap_or(i64::MIN);
            d.status_map
                .as_ref()
                .and_then(|map| map.iter().find(|(_, n)| *n == num).map(|(l, _)| l.clone()))
                .unwrap_or_else(|| v.as_str().unwrap_or("").to_string())
        }
        StatusMode::String => v.as_str().unwrap_or("").to_string(),
    };

    if !values.contains(&label) {
        return Err(anyhow::anyhow!(
            "status '{}': not one of [{}]",
            label,
            values.join(", ")
        ));
    }
    Ok(())
}

pub struct StatusableProtocol;

#[async_trait]
impl Protocol for StatusableProtocol {
    fn name(&self) -> &str {
        "statusable"
    }

    fn description(&self) -> &str {
        "Configurable status field supporting string and numeric mapping storage modes"
    }

    fn columns(&self) -> Vec<ColumnDef> {
        vec![ColumnDef {
            name: COL_STATUS.into(),
            sql_type: SqlType::Varchar,
            default: None,
        }]
    }

    fn behaviors(&self) -> Vec<&'static str> {
        vec!["statusable"]
    }

    fn apply_config(
        &self,
        config: &std::collections::HashMap<String, String>,
        decl: &mut ProtocolDeclaration,
        _all_columns: &[&str],
    ) {
        let mode = config.get("mode").is_some_and(|m| m == "numeric");
        let Some(values_str) = config.get("values") else {
            return;
        };

        if mode {
            let map: Vec<(String, i64)> = values_str
                .split(',')
                .filter_map(|pair| {
                    let mut parts = pair.trim().splitn(2, '=');
                    let label = parts.next()?.trim().to_string();
                    let num: i64 = parts.next()?.trim().parse().ok()?;
                    Some((label, num))
                })
                .collect();
            let labels: Vec<String> = map.iter().map(|(l, _)| l.clone()).collect();
            decl.status_values = Some(labels);
            decl.status_map = Some(map);
            decl.status_mode = StatusMode::Numeric;
        } else {
            let labels: Vec<String> = values_str
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            decl.status_values = Some(labels);
            decl.status_mode = StatusMode::String;
        }

        if let Some(default) = config.get("default") {
            decl.status_default = Some(default.clone());
        }
    }

    fn built_in(&self) -> bool {
        true
    }

    async fn before_create(
        &self,
        record: &mut serde_json::Map<String, Value>,
        ctx: &HookCtx<'_>,
    ) -> anyhow::Result<()> {
        if !ctx.schema.is_none_or(|s| s.is_protocol_column(COL_STATUS)) {
            return Ok(());
        }

        let decl = declaration_from_schema(ctx.schema);

        if !record.contains_key(COL_STATUS) {
            let default = decl
                .and_then(|d| d.status_default.as_deref())
                .unwrap_or("draft");
            let db_val = to_db_value(default, decl);
            record.insert(COL_STATUS.into(), db_val);
        }

        if let Some(v) = record.get(COL_STATUS) {
            validate_status(v, decl)?;
        }

        Ok(())
    }

    async fn before_update(
        &self,
        new_record: &mut serde_json::Map<String, Value>,
        _old_record: &serde_json::Map<String, Value>,
        ctx: &HookCtx<'_>,
    ) -> anyhow::Result<()> {
        if let Some(v) = new_record.get(COL_STATUS) {
            let decl = declaration_from_schema(ctx.schema);
            validate_status(v, decl)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_string_mode_values() {
        let mut decl = ProtocolDeclaration::default();
        let config = std::collections::HashMap::from([
            ("values".into(), "draft,published,archived".into()),
            ("default".into(), "draft".into()),
        ]);
        StatusableProtocol.apply_config(&config, &mut decl, &[]);
        assert_eq!(
            decl.status_values,
            Some(vec!["draft".into(), "published".into(), "archived".into()])
        );
        assert_eq!(decl.status_default, Some("draft".into()));
        assert_eq!(decl.status_mode, StatusMode::String);
    }

    #[test]
    fn parse_numeric_mode_values() {
        let mut decl = ProtocolDeclaration::default();
        let config = std::collections::HashMap::from([
            ("values".into(), "draft=1,published=10,archived=99".into()),
            ("default".into(), "1".into()),
            ("mode".into(), "numeric".into()),
        ]);
        StatusableProtocol.apply_config(&config, &mut decl, &[]);
        assert_eq!(
            decl.status_values,
            Some(vec!["draft".into(), "published".into(), "archived".into()])
        );
        assert_eq!(
            decl.status_map,
            Some(vec![
                ("draft".into(), 1),
                ("published".into(), 10),
                ("archived".into(), 99),
            ])
        );
        assert_eq!(decl.status_mode, StatusMode::Numeric);
    }

    #[tokio::test]
    async fn injects_default_status_on_create() {
        let protocol = StatusableProtocol;
        let mut record = serde_json::Map::new();
        let ctx = HookCtx {
            user_id: None,
            user_role: None,
            tenant_id: "default",
            now: "now",
            schema: None,
            pool: None,
        };

        protocol.before_create(&mut record, &ctx).await.unwrap();

        assert_eq!(record.get(COL_STATUS), Some(&json!("draft")));
    }

    #[test]
    fn provides_status_column() {
        let cols = StatusableProtocol.columns();
        assert_eq!(cols.len(), 1);
        assert_eq!(cols[0].name, COL_STATUS);
    }
}

crate::register_protocol!(
    crate::protocols::statusable::StatusableProtocol,
    crate::protocols::statusable::StatusableProtocol
);
