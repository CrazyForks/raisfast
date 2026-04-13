/// 宿主提供的运行时 API，供 JS 插件调用。
///
/// 通过全局变量 `Host` 访问，无需 import。
interface Host {
    /** 写入宿主日志。
     *
     * @param level - 日志级别：`"info"` | `"warn"` | `"error"`
     * @param message - 日志消息 */
    log(level: "info" | "warn" | "error", message: string): void;

    /** 读取宿主配置值。
     *
     * 支持的 key：
     * - `app.host` / `app.port` / `app.env` / `app.base_url`
     * - `jwt.access_expires` / `jwt.refresh_expires`
     * - `upload.dir` / `upload.max_size`
     * - `plugin.max_memory_mb` / `plugin.default_timeout_ms`
     *
     * @param key - 点分隔的配置路径
     * @returns 配置值字符串，不存在时返回 `null` */
    getConfig(key: string): string | null;
}

declare var Host: Host;

/// Hook 方法签名。
///
/// - **JSON Filter** — 接收 JSON 字符串，返回修改后的 JSON 字符串。
///   用于 `on_post_creating` / `on_post_updating` / `on_comment_creating`。
///
/// - **JSON Action** — 接收 JSON 字符串，无返回值。
///   用于 `on_post_created` / `on_post_updated` / `on_post_deleted` /
///   `on_comment_created` / `on_login`。
///
/// - **String Filter** — 接收原始字符串，返回修改后的字符串。
///   用于 `render_markdown` / `filter_html`。
///
/// - **Route Handler** — 接收 `{ path, method }` JSON 字符串，
///   返回 `{ status, body }` JSON 字符串。用于 `handle_route`。
interface PluginHooks {
    /** 文章创建前（Filter）：可修改 title / content / excerpt 等 */
    on_post_creating?(inputJson: string): string;

    /** 文章创建后（Action）：通知、索引、缓存清理等 */
    on_post_created?(dataJson: string): void;

    /** 文章更新前（Filter） */
    on_post_updating?(inputJson: string): string;

    /** 文章更新后（Action） */
    on_post_updated?(dataJson: string): void;

    /** 文章删除后（Action） */
    on_post_deleted?(dataJson: string): void;

    /** 评论创建前（Filter）：可过滤敏感词、修改内容 */
    on_comment_creating?(inputJson: string): string;

    /** 评论创建后（Action） */
    on_comment_created?(dataJson: string): void;

    /** 自定义 Markdown 渲染（String Filter）：替换默认渲染器 */
    render_markdown?(content: string): string;

    /** HTML 后处理（String Filter）：注入 meta 标签、OG 标签等 */
    filter_html?(html: string): string;

    /** 自定义路由（JSON Filter）：返回 `{ status: number, body: string }` */
    handle_route?(routeJson: string): string;

    /** 用户登录后（Action） */
    on_login?(dataJson: string): void;
}

/** 插件必须导出的全局对象，包含对应 Hook 方法。 */
declare var Plugin: PluginHooks;
