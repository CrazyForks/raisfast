// SEO Optimizer — Rhai 插件引擎验证
//
// 验证能力：
//   1. filter hook — on_post_creating: 自动填充 slug / meta_description
//   2. action hook — on_post_created: 写入 VFS 缓存
//   3. string filter — render_markdown: 向 <head> 注入 og:meta
//   4. custom route — seo_stats / seo_health
//   5. host API — getConfig / log / vfsWrite / vfsRead / vfsExists / newId

// ── filter hook: 自动生成 slug 和 meta_description ──────────────
fn on_post_creating(input) {
    // 自动生成 slug
    let title = input.title;
    input.slug = to_lower(replace(title, " ", "-")) + "-" + newId().sub_string(0, 8);

    // 自动生成 meta_description（截取 content 前 120 字符）
    let content = input.content;
    if content.len() > 120 {
        input.meta_description = content.sub_string(0, 120) + "...";
    } else {
        input.meta_description = content;
    }

    // 标记已处理
    input._seo_optimized = true;
    log("info", "[seo-rhai] on_post_creating: optimized");

    input
}

// ── action hook: 文章创建后写入 VFS 缓存 ────────────────────────
fn on_post_created(data) {
    let slug = data.slug;
    let cache_key = "cache/posts/" + slug + ".json";
    vfsWrite(cache_key, to_json(data));

    // 统计计数
    let stats_key = "cache/seo_stats.json";
    let existing = "0";
    if vfsExists(stats_key) {
        existing = vfsRead(stats_key);
    }
    let count = parse_int(existing) + 1;
    vfsWrite(stats_key, count.to_string());

    log("info", "[seo-rhai] on_post_created: cached " + slug);
}

// ── string filter: 向 HTML 注入 Open Graph meta ─────────────────
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

// ── custom route: 返回 SEO 处理统计 ──────────────────────────────
fn seo_stats(input) {
    let stats_key = "cache/seo_stats.json";
    let count = "0";
    if vfsExists(stats_key) {
        count = vfsRead(stats_key);
    }
    `{"total_optimized":${count},"engine":"rhai","version":"1.0.0"}`
}

// ── custom route: 健康检查 ───────────────────────────────────────
fn seo_health(input) {
    `{"status":"ok","engine":"rhai"}`
}
