//! CLI subcommands for skills: `raisfast skills import <dir>`.

use std::path::PathBuf;

use clap::Subcommand;

#[derive(Subcommand)]
pub enum SkillsAction {
    /// Import a SKILL.md skill directory into the local skills store
    Import {
        /// Source directory containing SKILL.md (+ optional scripts/references/assets)
        source: PathBuf,
        /// Target layer: platform | tenant
        #[arg(long, default_value = "platform")]
        layer: String,
        /// Tenant id (required when layer=tenant)
        #[arg(long)]
        tenant: Option<String>,
        /// Overwrite if the skill already exists
        #[arg(long)]
        force: bool,
    },
}

pub fn run(action: SkillsAction) -> anyhow::Result<()> {
    match action {
        SkillsAction::Import {
            source,
            layer,
            tenant,
            force,
        } => import(source, &layer, tenant.as_deref(), force),
    }
}

fn import(source: PathBuf, layer: &str, tenant: Option<&str>, force: bool) -> anyhow::Result<()> {
    let root = raisfast::agent::skills::skills_root();
    let outcome =
        raisfast::agent::skills::import::import_skill(&source, &root, layer, tenant, force)?;
    for w in &outcome.warnings {
        eprintln!("warning: {w}");
    }
    println!(
        "imported skill '{}' -> {}",
        outcome.name,
        outcome.dest.display()
    );
    if !outcome.warnings.is_empty() {
        println!("({} warnings)", outcome.warnings.len());
    }
    Ok(())
}
