//! Code generation commands.
//!
//! Parses `schema.sqlite.sql` with `sqlparser` and generates model `.rs` files.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use sqlparser::ast::{ColumnDef, ColumnOption, DataType, Statement, TableConstraint};
use sqlparser::dialect::GenericDialect;
use sqlparser::parser::Parser;

use serde::Serialize;

// ── Data structures ──────────────────────────────────────────────────

#[derive(Serialize)]
struct Column {
    name: String,
    rust_type: String,
}

#[derive(Serialize)]
struct Table {
    name: String,
    struct_name: String,
    columns: Vec<Column>,
    has_tenant_id: bool,
    has_timestamp: bool,
    singular: String,
}

// ── CLI entry ────────────────────────────────────────────────────────

pub fn run_model(tables: &[String], force: bool, dry_run: bool) -> anyhow::Result<()> {
    let schema_path = find_schema()?;
    let sql = fs::read_to_string(&schema_path)
        .map_err(|e| anyhow::anyhow!("failed to read {}: {e}", schema_path.display()))?;

    let parsed = parse_tables(&sql)?;

    let targets: Vec<&Table> = if tables.is_empty() {
        parsed.values().collect()
    } else {
        tables
            .iter()
            .map(|t| {
                let key = t.to_lowercase();
                parsed
                    .values()
                    .find(|tbl| tbl.name == key || tbl.struct_name.to_lowercase() == key)
                    .ok_or_else(|| anyhow::anyhow!("table not found: {t}"))
            })
            .collect::<Result<_, _>>()?
    };

    let models_dir = find_models_dir()?;

    let mut tera = tera::Tera::default();
    tera.add_raw_template(
        "model.rs",
        include_str!("../../templates/codegen/model.rs.tera"),
    )?;

    for table in &targets {
        let ctx = tera::Context::from_serialize(table)?;
        let code = tera.render("model.rs", &ctx)?;

        let file_path = models_dir.join(format!("{}.rs", table.name));

        if dry_run {
            println!("── {} ──", file_path.display());
            println!("{code}");
            println!();
            continue;
        }

        if file_path.exists() && !force {
            println!(
                "SKIP {} (exists, use --force to overwrite)",
                file_path.display()
            );
            continue;
        }

        fs::write(&file_path, &code)
            .map_err(|e| anyhow::anyhow!("failed to write {}: {e}", file_path.display()))?;
        println!("WROTE {}", file_path.display());
    }

    Ok(())
}

// ── Schema discovery ─────────────────────────────────────────────────

fn find_schema() -> anyhow::Result<PathBuf> {
    let candidates = [
        "migrations/sqlite/schema.sqlite.sql",
        "schema.sqlite.sql",
        "migrations/schema.sqlite.sql",
    ];
    for c in &candidates {
        if Path::new(c).exists() {
            return Ok(PathBuf::from(c));
        }
    }
    Err(anyhow::anyhow!(
        "schema.sqlite.sql not found (searched: {})",
        candidates.join(", ")
    ))
}

fn find_models_dir() -> anyhow::Result<PathBuf> {
    let dir = Path::new("src/models");
    if dir.exists() {
        return Ok(dir.to_path_buf());
    }
    Err(anyhow::anyhow!("src/models/ directory not found"))
}

// ── SQL parsing ──────────────────────────────────────────────────────

fn parse_tables(sql: &str) -> anyhow::Result<BTreeMap<String, Table>> {
    let dialect = GenericDialect;
    let statements =
        Parser::parse_sql(&dialect, sql).map_err(|e| anyhow::anyhow!("SQL parse error: {e}"))?;

    let mut tables = BTreeMap::new();

    for stmt in &statements {
        let Statement::CreateTable(ct) = stmt else {
            continue;
        };

        let table_name = ct
            .name
            .0
            .first()
            .and_then(|p| match p {
                sqlparser::ast::ObjectNamePart::Identifier(i) => Some(i.value.to_lowercase()),
                _ => None,
            })
            .unwrap_or_default();

        if table_name.is_empty() {
            continue;
        }

        let _pk_columns = collect_pk_columns(&ct.columns, &ct.constraints);
        let has_tenant_id = ct.columns.iter().any(|c| c.name.value == "tenant_id");
        let has_timestamp = ct.columns.iter().any(|c| {
            let n = c.name.value.as_str();
            (n == "created_at" || n == "updated_at") && matches!(c.data_type, DataType::Text)
        });

        let columns: Vec<Column> = ct
            .columns
            .iter()
            .map(|c| {
                let col_name = &c.name.value;
                let is_nullable = is_column_nullable(c);
                let rust_type = if col_name == "tenant_id" {
                    "Option<String>".to_string()
                } else {
                    map_type(&c.data_type, is_nullable, col_name)
                };
                Column {
                    name: col_name.clone(),
                    rust_type,
                }
            })
            .collect();

        let struct_name = singularize(&to_pascal_case(&table_name));
        let singular = singularize(&table_name);

        tables.insert(
            table_name.clone(),
            Table {
                name: table_name,
                struct_name,
                columns,
                has_tenant_id,
                has_timestamp,
                singular,
            },
        );
    }

    Ok(tables)
}

fn collect_pk_columns(columns: &[ColumnDef], constraints: &[TableConstraint]) -> Vec<String> {
    let mut pks = Vec::new();

    for c in columns {
        for opt in &c.options {
            if let ColumnOption::PrimaryKey(_) = opt.option {
                pks.push(c.name.value.clone());
            }
        }
    }

    for c in constraints {
        if let TableConstraint::PrimaryKey(pk) = c {
            for ic in &pk.columns {
                if let sqlparser::ast::Expr::Identifier(ident) = &ic.column.expr {
                    pks.push(ident.value.clone());
                }
            }
        }
    }

    pks
}

fn is_column_nullable(col: &ColumnDef) -> bool {
    for opt in &col.options {
        match &opt.option {
            ColumnOption::NotNull => return false,
            ColumnOption::PrimaryKey(_) => return false,
            _ => {}
        }
    }
    true
}

fn map_type(data_type: &DataType, nullable: bool, col_name: &str) -> String {
    let base = match data_type {
        DataType::Int(_) | DataType::Integer(_) | DataType::SmallInt(_) | DataType::BigInt(_) => {
            "i64"
        }
        DataType::Boolean => "bool",
        DataType::Real | DataType::Float(_) | DataType::Double(_) => "f64",
        DataType::Text | DataType::Varchar(_) | DataType::Char(_) | DataType::String(_)
            if (col_name == "created_at" || col_name == "updated_at") =>
        {
            "Timestamp"
        }
        DataType::Text | DataType::Varchar(_) | DataType::Char(_) | DataType::String(_) => "String",
        DataType::Blob(_) => "Vec<u8>",
        _ => "String",
    };

    if nullable {
        format!("Option<{base}>")
    } else {
        base.to_string()
    }
}

// ── Helpers ──────────────────────────────────────────────────────────

fn to_pascal_case(snake: &str) -> String {
    snake
        .split('_')
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                None => String::new(),
                Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
            }
        })
        .collect()
}

fn singularize(word: &str) -> String {
    if word.ends_with("ies") && word.len() > 3 {
        format!("{}y", &word[..word.len() - 3])
    } else if word.ends_with("ses") || word.ends_with("xes") || word.ends_with("zes") {
        word[..word.len() - 2].to_string()
    } else if word.ends_with('s') && !word.ends_with("ss") && word.len() > 1 {
        word[..word.len() - 1].to_string()
    } else {
        word.to_string()
    }
}
