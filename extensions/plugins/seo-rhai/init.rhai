// SEO Optimizer — Rhai plugin engine validation
//
// Demonstrates:
//   1. filter hook — on_post_creating: auto-fill slug / meta_description
//   2. action hook — on_post_created: write VFS cache
//   3. string filter — render_markdown: inject og:meta into <head>
//   4. custom route — seo_stats / seo_health
//   5. host API — getConfig / log / vfsWrite / vfsRead / vfsExists / newId

// ── filter hook: auto-generate slug and meta_description ───────
fn on_post_creating(input) {
    // Auto-generate slug
    let title = input.title;
    input.slug = to_lower(replace(title, " ", "-")) + "-" + newId().sub_string(0, 8);

    // Auto-generate meta_description (first 120 chars of content)
    let content = input.content;
    if content.len() > 120 {
        input.meta_description = content.sub_string(0, 120) + "...";
    } else {
        input.meta_description = content;
    }

    // Mark as processed
    input._seo_optimized = true;
    log("info", "[seo-rhai] on_post_creating: optimized");

    input
}

// ── action hook: write VFS cache after post creation ───────────
fn on_post_created(data) {
    let slug = data.slug;
    let cache_key = "cache/posts/" + slug + ".json";
    vfsWrite(cache_key, to_json(data));

    // Increment counter
    let stats_key = "cache/seo_stats.json";
    let existing = "0";
    if vfsExists(stats_key) {
        existing = vfsRead(stats_key);
    }
    let count = parse_int(existing) + 1;
    vfsWrite(stats_key, count.to_string());

    log("info", "[seo-rhai] on_post_created: cached " + slug);
}

// ── string filter: inject Open Graph meta into HTML ────────────
fn render_markdown(html) {
    let og_type = getConfig("seo.og_type");
    let og_str = "" + og_type;
    if og_str == "" || og_str == "()" {
        og_str = "article";
    }

    let injection = `<meta property="og:type" content="${og_str}">` +
        `<meta name="generator" content="raisfast-seo-rhai/1.0.0">`;

    let result = replace(html, "<head>", "<head>" + injection);
    log("info", "[seo-rhai] render_markdown: injected og meta");

    result
}

// ── custom route: return SEO processing stats ──────────────────
fn seo_stats(input) {
    let stats_key = "cache/seo_stats.json";
    let count = "0";
    if vfsExists(stats_key) {
        count = vfsRead(stats_key);
    }
    `{"total_optimized":${count},"engine":"rhai","version":"1.0.0"}`
}

// ── custom route: health check ─────────────────────────────────
fn seo_health(input) {
    `{"status":"ok","engine":"rhai"}`
}
