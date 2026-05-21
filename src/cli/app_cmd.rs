//! App CLI subcommand.
//!
//! Provides `app new` (generate project directory template).

use std::path::{Path, PathBuf};

pub fn create_new(name: &str, template: &str) -> anyhow::Result<()> {
    let project_dir = PathBuf::from(name);

    if project_dir.exists() {
        anyhow::bail!("directory already exists: {}", project_dir.display());
    }

    std::fs::create_dir_all(&project_dir)?;

    let dirs = [
        "extensions/content_types",
        "extensions/plugins",
        "migrations",
        "data",
        "logs",
        "public/uploads",
    ];

    for dir in &dirs {
        std::fs::create_dir_all(project_dir.join(dir))?;
    }

    let config_toml = render_template(include_str!("../../templates/app/config.toml"), name, "");
    std::fs::write(project_dir.join("config.toml"), config_toml)?;

    std::fs::write(
        project_dir.join(".env.example"),
        include_str!("../../templates/app/env.example"),
    )?;
    std::fs::write(
        project_dir.join(".gitignore"),
        include_str!("../../templates/app/gitignore"),
    )?;

    match template {
        "blog" => copy_content_types(&project_dir, "blog", &["article", "category", "tag"])?,
        "ecommerce" => copy_content_types(
            &project_dir,
            "ecommerce",
            &["product", "product_category", "order"],
        )?,
        _ => {}
    }

    let readme = render_template(
        include_str!("../../templates/app/README.md"),
        name,
        match template {
            "blog" => "A blog project built with raisfast",
            "ecommerce" => "An e-commerce project built with raisfast",
            _ => "A raisfast project",
        },
    );
    std::fs::write(project_dir.join("README.md"), readme)?;

    println!("✓ project created: {}", project_dir.display());
    println!();
    println!("  {name}/");
    println!("  ├── config.toml");
    println!("  ├── .env.example");
    println!("  ├── .gitignore");
    println!("  ├── extensions/");
    println!("  │   ├── content_types/");
    println!("  │   └── plugins/");
    println!("  ├── migrations/");
    println!("  ├── data/");
    println!("  ├── logs/");
    println!("  └── public/uploads/");
    println!();
    println!("  template: {template}");
    println!();
    println!("  cd {name}");
    println!("  cp .env.example .env");
    println!("  raisfast db migrate");
    println!("  raisfast db rollback");
    println!("  raisfast db seed");
    println!("  raisfast server start");

    Ok(())
}

fn render_template(template: &str, name: &str, description: &str) -> String {
    template
        .replace("{{ name }}", name)
        .replace("{{ description }}", description)
}

fn copy_content_types(project_dir: &Path, template: &str, files: &[&str]) -> anyhow::Result<()> {
    let ct_dir = project_dir.join("extensions/content_types");
    std::fs::create_dir_all(&ct_dir)?;

    for file in files {
        let content = match template {
            "blog" => match *file {
                "article" => include_str!("../../templates/app/blog/article.toml"),
                "category" => include_str!("../../templates/app/blog/category.toml"),
                "tag" => include_str!("../../templates/app/blog/tag.toml"),
                _ => continue,
            },
            "ecommerce" => match *file {
                "product" => include_str!("../../templates/app/ecommerce/product.toml"),
                "product_category" => {
                    include_str!("../../templates/app/ecommerce/product_category.toml")
                }
                "order" => include_str!("../../templates/app/ecommerce/order.toml"),
                _ => continue,
            },
            _ => continue,
        };
        std::fs::write(ct_dir.join(format!("{file}.toml")), content)?;
    }

    Ok(())
}
