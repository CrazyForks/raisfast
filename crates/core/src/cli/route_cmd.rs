//! `raisfast route` CLI subcommand.
//!
//! Inspect registered routes without starting the HTTP server.

use raisfast::config::app::AppConfig;
use raisfast::server::RouteRegistry;

fn collect_routes(config: &AppConfig) -> Vec<RouteInfo> {
    let mut registry = RouteRegistry::default();
    let _ = raisfast::handlers::auth::routes(&mut registry, config);
    let _ = raisfast::handlers::oauth::routes(&mut registry, config);
    let _ = raisfast::handlers::api_token::routes(&mut registry, config);
    let _ = raisfast::handlers::user::routes(&mut registry, config);
    let _ = raisfast::handlers::wallet::routes(&mut registry, config);
    let _ = raisfast::handlers::currencies::routes(&mut registry, config);

    if config.builtins.blog {
        let _ = raisfast::handlers::category::routes(&mut registry, config);
        let _ = raisfast::handlers::tag::routes(&mut registry, config);
        let _ = raisfast::handlers::post::routes(&mut registry, config);
        let _ = raisfast::handlers::post::admin_routes(&mut registry, config);
        let _ = raisfast::handlers::comment::routes(&mut registry, config);
    }

    if config.builtins.pages {
        let _ = raisfast::handlers::page::routes(&mut registry, config);
        let _ = raisfast::handlers::reusable_block::routes(&mut registry, config);
    }

    if config.builtins.media {
        let _ = raisfast::handlers::media::routes(0, &mut registry, config);
    }

    let _ = raisfast::handlers::sse::routes(&mut registry, config);
    let _ = raisfast::handlers::ws::routes(&mut registry, config);
    let _ = raisfast::graphql::handler::routes(&mut registry, config);

    let _ = raisfast::handlers::plugin::routes(&mut registry, config);
    let _ = raisfast::handlers::cron::routes(&mut registry, config);
    let _ = raisfast::handlers::rbac::routes(&mut registry, config);
    let _ = raisfast::handlers::stats::routes(&mut registry, config);
    let _ = raisfast::handlers::options::routes(&mut registry, config);
    let _ = raisfast::handlers::tenant::routes(&mut registry, config);
    let _ = raisfast::handlers::audit::routes(&mut registry, config);
    let _ = raisfast::webhook::handler::routes(&mut registry, config);
    let _ = raisfast::content_type::handler::routes(&mut registry, config);

    if config.builtins.workflow {
        let _ = raisfast::workflow::handler::routes(&mut registry, config);
    }

    let mut routes = registry.into_vec();
    routes.sort_by(|a, b| {
        a.source
            .cmp(&b.source)
            .then_with(|| a.path.cmp(&b.path))
            .then_with(|| a.method.cmp(&b.method))
    });
    routes
}

use clap::Subcommand;

#[derive(Subcommand)]
pub enum RouteAction {
    /// List all registered routes
    List {
        /// Filter by HTTP method (GET, POST, PUT, DELETE)
        #[arg(short, long)]
        method: Option<String>,
        /// Filter by path prefix (e.g. "/admin")
        #[arg(short, long)]
        prefix: Option<String>,
        /// Output format: table, json, csv
        #[arg(short = 'f', long, default_value = "table")]
        format: String,
    },
    /// Show routes for a specific module
    Show {
        /// Module name (e.g. "posts", "admin/users", "content_type")
        module: String,
    },
    /// Show route statistics summary
    Stats,
}

pub fn run(action: RouteAction, config: &AppConfig) {
    let routes = collect_routes(config);

    match action {
        RouteAction::List {
            method,
            prefix,
            format,
        } => list_routes(&routes, method.as_deref(), prefix.as_deref(), &format),
        RouteAction::Show { module } => show_module(&routes, &module),
        RouteAction::Stats => show_stats(&routes),
    }
}

fn list_routes(routes: &[RouteInfo], method: Option<&str>, prefix: Option<&str>, format: &str) {
    let filtered: Vec<&RouteInfo> = routes
        .iter()
        .filter(|r| {
            if let Some(m) = method
                && r.method.to_uppercase() != m.to_uppercase()
            {
                return false;
            }
            if let Some(p) = prefix
                && !r.path.starts_with(p)
            {
                return false;
            }
            true
        })
        .collect();

    match format {
        "json" => {
            let json = serde_json::to_string_pretty(&filtered).unwrap_or_default();
            println!("{json}");
        }
        "csv" => {
            println!("method,path,source,source_name");
            for r in &filtered {
                println!("{},{},{},{}", r.method, r.path, r.source, r.source_name);
            }
        }
        _ => print_table(&filtered),
    }
}

fn print_table(routes: &[&RouteInfo]) {
    if routes.is_empty() {
        println!("No routes found.");
        return;
    }

    let method_w = routes
        .iter()
        .map(|r| r.method.len())
        .max()
        .unwrap_or(6)
        .max(6);
    let path_w = routes
        .iter()
        .map(|r| r.path.len())
        .max()
        .unwrap_or(4)
        .max(4);
    let source_w = routes
        .iter()
        .map(|r| r.source.len())
        .max()
        .unwrap_or(6)
        .max(6);

    let bold = "\x1b[1m";
    let dim = "\x1b[2m";
    let reset = "\x1b[0m";
    let green = "\x1b[32m";
    let yellow = "\x1b[33m";
    let blue = "\x1b[34m";
    let magenta = "\x1b[35m";
    let cyan = "\x1b[36m";

    let method_colors = [
        ("GET", green),
        ("POST", yellow),
        ("PUT", blue),
        ("DELETE", magenta),
        ("PATCH", cyan),
    ];

    println!(
        "{bold}{:<method_w$} {:<path_w$} {:<source_w$} SOURCE_NAME{reset}",
        "METHOD",
        "PATH",
        "SOURCE",
        method_w = method_w,
        path_w = path_w,
        source_w = source_w,
    );
    println!(
        "{dim}{}{reset}",
        "-".repeat(method_w + path_w + source_w + 30)
    );

    for r in routes {
        let color = method_colors
            .iter()
            .find(|(m, _)| *m == r.method)
            .map(|(_, c)| *c)
            .unwrap_or(reset);
        println!(
            "{color}{:<method_w$}{reset} {:<path_w$} {:<source_w$} {}",
            r.method,
            r.path,
            r.source,
            r.source_name,
            method_w = method_w,
            path_w = path_w,
            source_w = source_w,
        );
    }

    println!();
    println!("{dim}{} route(s){reset}", routes.len());
}

fn show_module(routes: &[RouteInfo], module: &str) {
    let module_lower = module.to_lowercase();
    let filtered: Vec<&RouteInfo> = routes
        .iter()
        .filter(|r| {
            r.source_name.to_lowercase().contains(&module_lower)
                || r.source.to_lowercase().contains(&module_lower)
                || r.path.to_lowercase().contains(&module_lower)
        })
        .collect();

    if filtered.is_empty() {
        println!("No routes found for module: {module}");
        return;
    }

    println!("\x1b[1mRoutes for: {module}\x1b[0m");
    println!();

    // Group by path
    let mut paths: Vec<&RouteInfo> = filtered;
    paths.sort_by(|a, b| a.path.cmp(&b.path).then_with(|| a.method.cmp(&b.method)));

    print_table(&paths);
}

fn show_stats(routes: &[RouteInfo]) {
    let mut by_method: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut by_source: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut unique_paths = std::collections::HashSet::new();

    for r in routes {
        *by_method.entry(r.method.clone()).or_default() += 1;
        *by_source.entry(r.source_name.clone()).or_default() += 1;
        unique_paths.insert(r.path.clone());
    }

    let bold = "\x1b[1m";
    let reset = "\x1b[0m";
    let dim = "\x1b[2m";

    println!("{bold}Route Statistics{reset}");
    println!("{dim}{}{reset}", "-".repeat(40));

    println!();
    println!("{bold}By Method:{reset}");
    let mut methods: Vec<_> = by_method.iter().collect();
    methods.sort_by(|a, b| b.1.cmp(a.1));
    for (method, count) in &methods {
        println!("  {:<10} {}", method, count);
    }

    println!();
    println!("{bold}By Module:{reset}");
    let mut sources: Vec<_> = by_source.iter().collect();
    sources.sort_by(|a, b| b.1.cmp(a.1));
    for (source, count) in &sources {
        println!("  {:<25} {}", source, count);
    }

    println!();
    println!("{bold}Summary:{reset}");
    println!("  Total route entries:  {}", routes.len());
    println!("  Unique paths:         {}", unique_paths.len());
    println!("  Modules:              {}", sources.len());
}

use raisfast::server::RouteInfo;
