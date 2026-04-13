//! 内容过滤插件
//!
//! 过滤评论中的敏感词，替换为 ***。

use rust_blog_plugin_sdk::*;

const SENSITIVE_WORDS: &[&str] = &["badword1", "badword2", "spam_link"];

/// 评论创建前过滤器：过滤敏感词
#[no_mangle]
pub extern "C" fn on_comment_creating(ptr: i32, len: i32) -> i32 {
    let mut input: CommentInput = read_input(ptr, len);
    input.content = filter_sensitive(&input.content);
    if let Some(nick) = &input.nickname {
        input.nickname = Some(filter_sensitive(nick));
    }
    write_output(&input)
}

fn filter_sensitive(text: &str) -> String {
    let mut result = text.to_string();
    for word in SENSITIVE_WORDS {
        let replacement = "*".repeat(word.len());
        result = result.replace(word, &replacement);
    }
    result
}
