pub mod batch;
pub mod cart;
pub mod category;
pub mod comment;
pub mod currencies;
pub mod ecommerce;
pub mod media;
pub mod order;
pub mod page;
pub mod payment;
pub mod post;
pub mod reusable_block;
pub mod tag;
pub mod tenant;
pub mod user;
pub mod wallet;

pub use batch::*;
pub use cart::*;
pub use category::*;
pub use comment::*;
pub use currencies::*;
pub use ecommerce::*;
pub use media::*;
pub use order::*;
pub use page::*;
pub use payment::*;
pub use post::*;
pub use reusable_block::*;
pub use tag::*;
pub use tenant::*;
pub use user::*;
pub use wallet::*;

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

fn validate_optional_uuid(id: &str) -> Result<(), validator::ValidationError> {
    if id.parse::<uuid::Uuid>().is_err() && id.parse::<i64>().is_err() {
        let mut err = validator::ValidationError::new("invalid_id");
        err.message = Some("invalid ID format".into());
        return Err(err);
    }
    Ok(())
}

fn validate_uuid_vec(ids: &[String]) -> Result<(), validator::ValidationError> {
    for id in ids {
        if id.parse::<uuid::Uuid>().is_err() && id.parse::<i64>().is_err() {
            let mut err = validator::ValidationError::new("invalid_id");
            err.message = Some("invalid ID format".into());
            return Err(err);
        }
    }
    Ok(())
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
    fn validate_optional_uuid_valid_uuid() {
        assert!(validate_optional_uuid("01901234-5678-7000-8000-000000000000").is_ok());
    }

    #[test]
    fn validate_optional_uuid_valid_i64() {
        assert!(validate_optional_uuid("42").is_ok());
    }

    #[test]
    fn validate_optional_uuid_invalid() {
        assert!(validate_optional_uuid("not-a-uuid").is_err());
    }

    #[test]
    fn validate_uuid_vec_valid() {
        assert!(
            validate_uuid_vec(&[
                "01901234-5678-7000-8000-000000000000".to_string(),
                "1".to_string()
            ])
            .is_ok()
        );
    }

    #[test]
    fn validate_uuid_vec_invalid() {
        assert!(validate_uuid_vec(&["bad-id".to_string()]).is_err());
    }
}
