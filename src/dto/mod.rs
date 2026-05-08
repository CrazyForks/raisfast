pub mod category;
pub mod comment;
pub mod media;
pub mod post;
pub mod tag;
pub mod user;

pub use category::*;
pub use comment::*;
pub use media::*;
pub use post::*;
pub use tag::*;
pub use user::*;

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
