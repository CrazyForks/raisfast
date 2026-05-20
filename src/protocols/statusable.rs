//! statusable Protocol — configurable status field
//!
//! Supports 2 storage modes:
//! - String mode (default): DB stores `"draft"` / `"published"`
//! - Numeric mapping mode: DB stores `1` / `10` / `99`, API interacts with strings
//!
//! Query filtering is not handled by the protocol but by the API rule engine (`[api.list] filter = 'status = "published"'`).

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::aspects::{
    Advice, Aspect, AspectResult, ColumnDef, DataBeforeCreateContext, DataBeforeUpdateContext,
    Layer, Operation, Pointcut, SqlType, TargetMatcher, When,
};
use crate::constants::COL_STATUS;
use crate::protocols::{Protocol, ProtocolDeclaration, StatusMode};

pub struct StatusableAspect;

#[async_trait]
impl Aspect for StatusableAspect {
    fn name(&self) -> &str {
        "statusable"
    }

    fn priority(&self) -> i32 {
        -150
    }

    fn pointcuts(&self) -> Vec<Pointcut> {
        vec![
            Pointcut {
                layer: Layer::Data,
                operation: Operation::Create,
                when: When::Before,
                target: TargetMatcher::All,
            },
            Pointcut {
                layer: Layer::Data,
                operation: Operation::Update,
                when: When::Before,
                target: TargetMatcher::All,
            },
        ]
    }

    fn columns(&self) -> Vec<ColumnDef> {
        vec![ColumnDef {
            name: COL_STATUS.into(),
            sql_type: SqlType::Varchar,
            default: None,
        }]
    }

    async fn on_data_before_create(&self, ctx: &mut DataBeforeCreateContext) -> AspectResult {
        if !ctx
            .schema
            .as_ref()
            .is_none_or(|s| s.is_protocol_column(COL_STATUS))
        {
            return Ok(Advice::Continue);
        }

        let decl = ctx
            .schema
            .as_ref()
            .and_then(|s| s.cached_declaration.as_ref());

        if !ctx.record.contains_key(COL_STATUS) {
            let default = decl
                .and_then(|d| d.status_default.as_deref())
                .unwrap_or("draft");
            let db_val = to_db_value(default, decl);
            ctx.record.insert(COL_STATUS.into(), db_val);
        }

        if let Some(v) = ctx.record.get(COL_STATUS) {
            validate_status(v, decl)?;
        }

        Ok(Advice::Continue)
    }

    async fn on_data_before_update(&self, ctx: &mut DataBeforeUpdateContext) -> AspectResult {
        if let Some(v) = ctx.new_record.get(COL_STATUS) {
            let decl = ctx
                .schema
                .as_ref()
                .and_then(|s| s.cached_declaration.as_ref());
            validate_status(v, decl)?;
        }
        Ok(Advice::Continue)
    }
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

impl Protocol for StatusableProtocol {
    fn name(&self) -> &str {
        "statusable"
    }

    fn description(&self) -> &str {
        "Configurable status field supporting string and numeric mapping storage modes"
    }

    fn aspects(&self) -> Vec<Arc<dyn Aspect>> {
        vec![Arc::new(StatusableAspect)]
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aspects::engine::AspectEngine;
    use crate::aspects::{BaseContext, Record};

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
        let engine = AspectEngine::new();
        engine.register(StatusableAspect);

        let mut ctx = DataBeforeCreateContext {
            base: BaseContext::new(None, "default".into(), "now".into()),
            table: "posts".into(),
            record: Record::new(),
            schema: None,
        };

        engine
            .dispatch_data_before_create("posts", &mut ctx)
            .await
            .unwrap();

        assert_eq!(ctx.record.get(COL_STATUS), Some(&json!("draft")));
    }

    #[test]
    fn provides_status_column() {
        let cols = StatusableAspect.columns();
        assert_eq!(cols.len(), 1);
        assert_eq!(cols[0].name, COL_STATUS);
    }
}

crate::register_protocol!(
    crate::protocols::statusable::StatusableProtocol,
    crate::protocols::statusable::StatusableProtocol
);
