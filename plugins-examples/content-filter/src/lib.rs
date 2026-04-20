//! 内容过滤插件（Component Model 版本）

rust_blog_plugin_sdk::wit_bindgen::generate!({
    path: "../../plugins-protocol/wit/plugin.wit",
    world: "plugin-world",
});

use exports::rust_blog::plugin_protocol::plugin_hooks::{
    CommentInput, ContentEvent, Guest, PostInput, PostOutput,
};

const SENSITIVE_WORDS: &[&str] = &["badword1", "badword2", "spam_link"];

struct ContentFilter;

impl Guest for ContentFilter {
    fn on_post_creating(_input: PostInput) -> Option<PostInput> {
        None
    }
    fn on_post_updating(_input: PostInput) -> Option<PostInput> {
        None
    }

    fn on_comment_creating(input: CommentInput) -> Option<CommentInput> {
        let mut input = input;
        input.content = filter_sensitive(&input.content);
        if let Some(nick) = &input.nickname {
            input.nickname = Some(filter_sensitive(nick));
        }
        Some(input)
    }

    fn on_content_creating(_input: ContentEvent) -> Option<ContentEvent> {
        None
    }
    fn on_content_updating(_input: ContentEvent) -> Option<ContentEvent> {
        None
    }
    fn render_markdown(_input: String) -> Option<String> {
        None
    }
    fn filter_html(_input: String) -> Option<String> {
        None
    }
    fn on_post_created(_output: PostOutput) {}
    fn on_post_updated(_output: PostOutput) {}
    fn on_post_deleted(_id: String) {}
    fn on_comment_created(_input: CommentInput) {}
    fn on_content_created(_input: ContentEvent) {}
    fn on_content_updated(_input: ContentEvent) {}
    fn on_content_deleted(_content_type: String, _id: String) {}
    fn on_login(_user_id: String) {}
    fn on_cron_tick(_payload: Option<String>) {}
}

export!(ContentFilter);

fn filter_sensitive(text: &str) -> String {
    let mut result = text.to_string();
    for word in SENSITIVE_WORDS {
        let replacement = "*".repeat(word.len());
        result = result.replace(word, &replacement);
    }
    result
}
