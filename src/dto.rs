pub mod api_token;
pub mod audit;
pub mod batch;
pub mod cart;
pub mod category;
pub mod comment;
pub mod cron;
pub mod currencies;
pub mod ecommerce;
pub mod health;
pub mod media;
pub mod oauth;
pub mod options;
pub mod order;
pub mod page;
pub mod payment;
pub mod post;
pub mod reusable_block;
pub mod sse;
pub mod stats;
pub mod tag;
pub mod tenant;
pub mod user;
pub mod wallet;
pub mod ws;

pub use api_token::*;
pub use audit::*;
pub use batch::*;
pub use cart::*;
pub use category::*;
pub use comment::*;
pub use cron::*;
pub use currencies::*;
pub use ecommerce::*;
pub use health::*;
pub use media::*;
pub use oauth::*;
pub use options::*;
pub use order::*;
pub use page::*;
pub use payment::*;
pub use post::*;
pub use reusable_block::*;
pub use sse::*;
pub use stats::*;
pub use tag::*;
pub use tenant::*;
pub use user::*;
pub use wallet::*;
pub use ws::*;

fn validate_password(pwd: &str) -> Result<(), validator::ValidationError> {
    let has_letter = pwd.chars().any(|c| c.is_ascii_alphabetic());
    let has_digit = pwd.chars().any(|c| c.is_ascii_digit());
    if has_letter && has_digit {
        Ok(())
    } else {
        let mut err = validator::ValidationError::new("password_strength");
        err.message = Some("password must contain both letters and digits".into());
        Err(err)
    }
}

#[allow(dead_code)]
fn validate_post_status(status: &str) -> Result<(), validator::ValidationError> {
    match status {
        "draft" | "published" => Ok(()),
        _ => {
            let mut err = validator::ValidationError::new("invalid_status");
            err.message = Some("status must be 'draft' or 'published'".into());
            Err(err)
        }
    }
}

#[allow(dead_code)]
fn validate_comment_status(status: &str) -> Result<(), validator::ValidationError> {
    match status {
        "approved" | "pending" | "spam" => Ok(()),
        _ => {
            let mut err = validator::ValidationError::new("invalid_status");
            err.message = Some("status must be 'approved', 'pending', or 'spam'".into());
            Err(err)
        }
    }
}

fn validate_optional_id(id: &str) -> Result<(), validator::ValidationError> {
    if !is_valid_id_str(id) {
        let mut err = validator::ValidationError::new("invalid_id");
        err.message = Some("invalid ID format".into());
        return Err(err);
    }
    Ok(())
}

fn validate_id_vec(ids: &[String]) -> Result<(), validator::ValidationError> {
    for id in ids {
        if !is_valid_id_str(id) {
            let mut err = validator::ValidationError::new("invalid_id");
            err.message = Some("invalid ID format".into());
            return Err(err);
        }
    }
    Ok(())
}

fn is_valid_id_str(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric())
}

fn validate_currency_code(code: &str) -> Result<(), validator::ValidationError> {
    let valid =
        !code.is_empty() && code.len() <= 10 && code.chars().all(|c| c.is_ascii_uppercase());
    if valid {
        Ok(())
    } else {
        let mut err = validator::ValidationError::new("invalid_currency_code");
        err.message = Some("currency must be 1-10 uppercase ASCII letters".into());
        Err(err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_password_valid() {
        assert!(validate_password("abc123").is_ok());
        assert!(validate_password("Password1").is_ok());
    }

    #[test]
    fn validate_password_no_digit() {
        assert!(validate_password("abcdef").is_err());
    }

    #[test]
    fn validate_password_no_letter() {
        assert!(validate_password("123456").is_err());
    }

    #[test]
    fn validate_post_status_valid() {
        assert!(validate_post_status("draft").is_ok());
        assert!(validate_post_status("published").is_ok());
    }

    #[test]
    fn validate_post_status_invalid() {
        assert!(validate_post_status("archived").is_err());
    }

    #[test]
    fn validate_comment_status_valid() {
        assert!(validate_comment_status("approved").is_ok());
        assert!(validate_comment_status("pending").is_ok());
        assert!(validate_comment_status("spam").is_ok());
    }

    #[test]
    fn validate_comment_status_invalid() {
        assert!(validate_comment_status("deleted").is_err());
    }

    #[test]
    fn validate_optional_id_valid() {
        assert!(validate_optional_id("42").is_ok());
    }

    #[test]
    fn validate_optional_id_invalid() {
        assert!(validate_optional_id("!!!invalid!!!").is_err());
    }

    #[test]
    fn validate_id_vec_valid() {
        assert!(validate_id_vec(&["1".to_string(), "999".to_string()]).is_ok());
    }

    #[test]
    fn validate_id_vec_invalid() {
        assert!(validate_id_vec(&["!!!bad!!!".to_string()]).is_err());
    }
}
