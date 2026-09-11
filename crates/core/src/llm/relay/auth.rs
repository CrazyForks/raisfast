//! sk- token authentication for `/v1` relay (design §9.1, §5.2).

use std::net::Ipv4Addr;
use std::str::FromStr;

use crate::db::Pool;
use crate::errors::app_error::{AppError, AppResult};
use crate::llm::models::token::{self, LlmToken, LlmTokenStatus};
use crate::types::snowflake_id::SnowflakeId;

/// Authenticated relay identity.
pub struct RelayIdentity {
    pub token: LlmToken,
}

/// Generate a fresh downstream key: `sk-` + 48 url-safe chars (shown once).
pub fn generate_sk() -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut raw = [0u8; 48];
    let _ = getrandom::fill(&mut raw);
    let body: String = raw
        .iter()
        .map(|b| ALPHABET[*b as usize % ALPHABET.len()] as char)
        .collect();
    format!("sk-{body}")
}

/// Hash a plaintext sk- key for storage/lookup.
pub fn hash_sk(plain: &str) -> String {
    crate::services::api_token::hash_token(plain)
}

fn ip_allowed(allowed: Option<&str>, client_ip: &str) -> bool {
    let Some(list) = allowed else { return true };
    if list.trim().is_empty() {
        return true;
    }
    let client = client_ip.trim();
    for entry in list.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if let Some((net, bits)) = entry.split_once('/') {
            if ipv4_in_cidr(client, net, bits) {
                return true;
            }
        } else if entry == client {
            return true;
        }
    }
    false
}

fn ipv4_in_cidr(ip: &str, net: &str, bits: &str) -> bool {
    let Ok(bits) = bits.parse::<u32>() else {
        return false;
    };
    if bits > 32 {
        return false;
    }
    let Ok(a) = Ipv4Addr::from_str(ip) else {
        return false;
    };
    let Ok(b) = Ipv4Addr::from_str(net) else {
        return false;
    };
    let mask: u32 = if bits == 0 {
        0
    } else {
        u32::MAX << (32 - bits)
    };
    u32::from(a) & mask == u32::from(b) & mask
}

/// Full token authentication chain (design §9.1): hash lookup → owner user
/// status → token status (lazy expired/exhausted flips) → IP allowlist.
/// `client_ip` may be empty when no address is known (allowlist skipped).
pub async fn authenticate(pool: &Pool, bearer: &str, client_ip: &str) -> AppResult<RelayIdentity> {
    let plain = bearer.strip_prefix("sk-").unwrap_or(bearer);
    let full = if bearer.starts_with("sk-") {
        bearer.to_owned()
    } else {
        format!("sk-{plain}")
    };
    let token = token::find_by_hash(pool, &hash_sk(&full))
        .await?
        .ok_or(AppError::Unauthorized)?;

    // Owner user status: a banned user's tokens die with the account.
    if let Some(user) =
        crate::models::user::find_by_id(pool, token.user_id, token.tenant_id.as_deref()).await?
    {
        if user.status != crate::models::user::UserStatus::Active {
            return Err(AppError::Unauthorized);
        }
    } else {
        return Err(AppError::Unauthorized);
    }

    // Lazy status flips (design §9.2).
    if token.status == LlmTokenStatus::Enabled {
        let now = crate::utils::tz::now_utc();
        if let Some(exp) = token.expired_at
            && exp <= now
        {
            token::update_status(pool, None, token.id, LlmTokenStatus::Expired).await?;
            return Err(AppError::Unauthorized);
        }
        if !token.unlimited_quota && token.remain_quota.0 <= 0 {
            token::update_status(pool, None, token.id, LlmTokenStatus::Exhausted).await?;
            return Err(AppError::Unauthorized);
        }
    } else if token.status != LlmTokenStatus::Enabled {
        return Err(AppError::Unauthorized);
    }

    if !client_ip.is_empty() && !ip_allowed(token.allowed_ips.as_deref(), client_ip) {
        return Err(AppError::Forbidden);
    }

    Ok(RelayIdentity { token })
}

/// Model whitelist check (§5.2 `allowed_models`).
pub fn model_allowed(token: &LlmToken, model: &str) -> bool {
    match token.allowed_models.as_deref() {
        None | Some("") => true,
        Some(list) => list.split(',').map(str::trim).any(|m| m == model),
    }
}

/// Placeholder owner helper for log rows.
pub fn token_owner(token: &LlmToken) -> (SnowflakeId, SnowflakeId) {
    (token.id, token.user_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sk_format() {
        let sk = generate_sk();
        assert!(sk.starts_with("sk-"), "prefix");
        assert_eq!(sk.len(), 3 + 48, "48 random chars");
        assert!(sk[3..].chars().all(|c| c.is_ascii_alphanumeric()));
        assert_ne!(generate_sk(), sk, "randomness");
    }

    #[test]
    fn ip_allowlist_matching() {
        assert!(ip_allowed(None, "1.2.3.4"));
        assert!(ip_allowed(Some(""), "1.2.3.4"));
        assert!(ip_allowed(Some("1.2.3.4\n5.6.7.8"), "5.6.7.8"));
        assert!(ip_allowed(Some("10.0.0.0/8"), "10.1.2.3"));
        assert!(ip_allowed(Some("192.168.1.0/24"), "192.168.1.99"));
        assert!(!ip_allowed(Some("192.168.1.0/24"), "192.168.2.1"));
        assert!(!ip_allowed(Some("1.2.3.4"), "5.6.7.8"));
        assert!(!ip_allowed(Some("10.0.0.0/99"), "10.0.0.1"), "invalid bits");
    }

    #[test]
    fn model_whitelist() {
        let mut token = token_fixture();
        assert!(model_allowed(&token, "anything"));
        token.allowed_models = Some("gpt-4o, gpt-4o-mini".to_owned());
        assert!(model_allowed(&token, "gpt-4o"));
        assert!(!model_allowed(&token, "claude"));
    }

    fn token_fixture() -> crate::llm::models::token::LlmToken {
        use crate::llm::models::token::{LlmToken, LlmTokenStatus};
        LlmToken {
            id: SnowflakeId(1),
            tenant_id: Some("default".to_owned()),
            user_id: SnowflakeId(2),
            name: "t".to_owned(),
            key_hash: "h".to_owned(),
            key_enc: None,
            status: LlmTokenStatus::Enabled,
            remain_quota: crate::types::quota::Quota(100),
            used_quota: crate::types::quota::Quota(0),
            unlimited_quota: false,
            expired_at: None,
            allowed_models: None,
            allowed_ips: None,
            token_group: None,
            created_at: crate::utils::tz::now_utc(),
            accessed_at: None,
            updated_at: crate::utils::tz::now_utc(),
        }
    }
}
