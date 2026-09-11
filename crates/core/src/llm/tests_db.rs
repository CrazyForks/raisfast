//! DB-backed tests for the llm module: relay auth chain, quota billing and
//! key-state persistence — each test gets its own SQLite in-memory schema
//! via `crate::test_pool!()`.

use crate::commands::CreateUserCmd;
use crate::llm::models::token::{self, LlmTokenStatus};
use crate::llm::relay::auth;
use crate::models::user::{self, UserStatus};
use crate::types::snowflake_id::SnowflakeId;

async fn pool() -> crate::db::Pool {
    crate::test_pool!()
}

async fn make_user(p: &crate::db::Pool, status: UserStatus) -> SnowflakeId {
    let username = format!("llm-test-{}", crate::utils::id::new_id());
    let cmd = CreateUserCmd::new(username, crate::models::user::RegisteredVia::Email);
    let u = user::create(p, &cmd, None).await.expect("user");
    if status != UserStatus::Active {
        user::update_status(p, u.id, status, None)
            .await
            .expect("status");
    }
    u.id
}

async fn make_token(
    p: &crate::db::Pool,
    user_id: SnowflakeId,
    remain: i64,
    unlimited: bool,
) -> (String, crate::llm::models::token::LlmToken) {
    let plain = auth::generate_sk();
    let t = token::create_token(
        p,
        None,
        user_id,
        "test",
        &crate::services::api_token::hash_token(&plain),
        remain,
        unlimited,
        None,
        None,
        None,
    )
    .await
    .expect("token");
    (plain, t)
}

#[tokio::test]
async fn auth_full_chain_happy_path() {
    let p = pool().await;
    let uid = make_user(&p, UserStatus::Active).await;
    let (plain, _t) = make_token(&p, uid, 100, false).await;
    let id = auth::authenticate(&p, &plain, "1.2.3.4")
        .await
        .expect("auth ok");
    assert_eq!(id.token.user_id, uid);
}

#[tokio::test]
async fn auth_rejects_unknown_key_and_banned_owner() {
    let p = pool().await;
    let uid = make_user(&p, UserStatus::Active).await;
    let (_plain, _t) = make_token(&p, uid, 100, false).await;
    assert!(
        auth::authenticate(&p, "sk-does-not-exist", "")
            .await
            .is_err()
    );

    let banned = make_user(&p, UserStatus::Banned).await;
    let (plain_b, _tb) = make_token(&p, banned, 100, false).await;
    assert!(
        auth::authenticate(&p, &plain_b, "").await.is_err(),
        "banned owner kills the token (§9.1)"
    );
}

#[tokio::test]
async fn auth_lazy_flips_expired_and_exhausted() {
    let p = pool().await;
    let uid = make_user(&p, UserStatus::Active).await;

    // Expired: expired_at in the past → 401 + status flipped persistently.
    let (plain_e, t_e) = make_token(&p, uid, 100, false).await;
    let past = crate::utils::tz::now_utc() - chrono::Duration::seconds(10);
    let sql = format!(
        "UPDATE llm_tokens SET expired_at = {} WHERE id = {}",
        crate::db::Driver::ph(1),
        crate::db::Driver::ph(2)
    );
    use crate::db::driver::DbDriver;
    sqlx::query(crate::db::safe_sql(&sql))
        .bind(past)
        .bind(t_e.id)
        .execute(&p)
        .await
        .expect("set expired");
    assert!(auth::authenticate(&p, &plain_e, "").await.is_err());
    let after = token::find_by_id(&p, t_e.id, None).await.unwrap().unwrap();
    assert_eq!(after.status, LlmTokenStatus::Expired, "lazy flip");

    // Exhausted: zero remaining quota → 401 + flipped.
    let (plain_x, t_x) = make_token(&p, uid, 0, false).await;
    assert!(auth::authenticate(&p, &plain_x, "").await.is_err());
    let after_x = token::find_by_id(&p, t_x.id, None).await.unwrap().unwrap();
    assert_eq!(after_x.status, LlmTokenStatus::Exhausted);
}

#[tokio::test]
async fn auth_ip_allowlist_enforced() {
    let p = pool().await;
    let uid = make_user(&p, UserStatus::Active).await;
    let plain = auth::generate_sk();
    token::create_token(
        &p,
        None,
        uid,
        "ip",
        &crate::services::api_token::hash_token(&plain),
        100,
        false,
        None,
        None,
        Some("10.1.0.0/16\n127.0.0.1"),
    )
    .await
    .expect("token");
    assert!(auth::authenticate(&p, &plain, "10.1.2.3").await.is_ok());
    assert!(auth::authenticate(&p, &plain, "127.0.0.1").await.is_ok());
    assert!(auth::authenticate(&p, &plain, "8.8.8.8").await.is_err());
}

#[tokio::test]
async fn billing_preconsume_settle_refund_cycle() {
    let p = pool().await;
    let uid = make_user(&p, UserStatus::Active).await;
    let (_plain, t) = make_token(&p, uid, 1000, false).await;

    // Pre-consume 300 → remain 700.
    let charge = crate::llm::relay::billing::pre_consume(&p, t.id, false, 300)
        .await
        .expect("hold");
    assert_eq!(charge.pre_consumed, 300);
    let mid = token::find_by_id(&p, t.id, None).await.unwrap().unwrap();
    assert_eq!(mid.remain_quota, 700);

    // Actual 100 → refund 200 → remain 900, used tracks the actual 100.
    crate::llm::relay::billing::settle(&p, &charge, 100).await;
    let after = token::find_by_id(&p, t.id, None).await.unwrap().unwrap();
    assert_eq!(after.remain_quota, 900);
    assert_eq!(after.used_quota, 100);

    // Full refund restores to the original hold.
    let charge2 = crate::llm::relay::billing::pre_consume(&p, t.id, false, 250)
        .await
        .expect("hold2");
    crate::llm::relay::billing::refund_all(&p, &charge2).await;
    let final_row = token::find_by_id(&p, t.id, None).await.unwrap().unwrap();
    assert_eq!(final_row.remain_quota, 900);
}

#[tokio::test]
async fn billing_preconsume_rejects_insufficient() {
    let p = pool().await;
    let uid = make_user(&p, UserStatus::Active).await;
    let (_plain, t) = make_token(&p, uid, 50, false).await;
    let err = crate::llm::relay::billing::pre_consume(&p, t.id, false, 300)
        .await
        .expect_err("must reject");
    assert!(matches!(
        err,
        crate::errors::app_error::AppError::TooManyRequests(_)
    ));
    // Overdraw protection: unchanged balance.
    let row = token::find_by_id(&p, t.id, None).await.unwrap().unwrap();
    assert_eq!(row.remain_quota, 50);
}

#[tokio::test]
async fn billing_under_hold_top_up_collects_or_logs() {
    let p = pool().await;
    let uid = make_user(&p, UserStatus::Active).await;
    let (_plain, t) = make_token(&p, uid, 500, false).await;
    let charge = crate::llm::relay::billing::pre_consume(&p, t.id, false, 100)
        .await
        .expect("hold");
    // Actual 150 > hold 100 → collects the 50 diff.
    crate::llm::relay::billing::settle(&p, &charge, 150).await;
    let row = token::find_by_id(&p, t.id, None).await.unwrap().unwrap();
    assert_eq!(row.remain_quota, 350);
}

#[tokio::test]
async fn billing_unlimited_token_skips_holds() {
    let p = pool().await;
    let uid = make_user(&p, UserStatus::Active).await;
    let (_plain, t) = make_token(&p, uid, 0, true).await;
    let charge = crate::llm::relay::billing::pre_consume(&p, t.id, true, 12345)
        .await
        .expect("no hold for unlimited");
    crate::llm::relay::billing::settle(&p, &charge, 999).await;
    crate::llm::relay::billing::refund_all(&p, &charge).await;
    let row = token::find_by_id(&p, t.id, None).await.unwrap().unwrap();
    assert_eq!(row.remain_quota, 0, "unlimited never touches remain_quota");
}

#[tokio::test]
async fn key_status_persists_in_pool_json() {
    let p = pool().await;
    use crate::llm::models::channel::{self, LlmKeyStatus, NewChannel};
    let entries = vec![crate::llm::models::channel::LlmKeyEntry {
        key: "enc:v1:fake".to_owned(),
        status: LlmKeyStatus::Active,
        disabled_reason: None,
        disabled_at: None,
        max_concurrency: None,
    }];
    let row = channel::create_channel(
        &p,
        None,
        NewChannel {
            name: format!("ch-{}", crate::utils::id::new_id()),
            provider: "openai".to_owned(),
            base_url: "https://x.test/v1".to_owned(),
            api_keys: channel::keys_value(&entries),
            key_mode: crate::llm::models::channel::LlmKeyMode::Polling,
            models: "m1".to_owned(),
            model_mapping: None,
            priority: 0,
            weight: 0,
            channel_groups: "default".to_owned(),
            auto_ban: true,
            param_override: None,
            header_override: None,
            config: None,
            test_model: None,
        },
    )
    .await
    .expect("channel");

    let updated = channel::update_key_status(
        &p,
        None,
        row.id,
        0,
        LlmKeyStatus::Disabled,
        Some("401 unauthorized"),
    )
    .await
    .expect("update key");
    assert_eq!(updated[0].status, LlmKeyStatus::Disabled);
    assert_eq!(
        updated[0].disabled_reason.as_deref(),
        Some("401 unauthorized")
    );

    // Reload from DB: the JSON pool persisted the mutation.
    let reloaded = channel::find_by_id(&p, row.id, None)
        .await
        .unwrap()
        .unwrap();
    let entries = channel::parse_keys(&reloaded);
    assert_eq!(entries[0].status, LlmKeyStatus::Disabled);
}

#[tokio::test]
async fn log_insert_roundtrip() {
    let p = pool().await;
    let uid = make_user(&p, UserStatus::Active).await;
    let (_plain, t) = make_token(&p, uid, 100, false).await;
    crate::llm::models::log::insert_log(
        &p,
        crate::llm::models::log::NewLog {
            tenant_id: Some("default".to_owned()),
            user_id: Some(uid),
            token_id: Some(t.id),
            source: crate::llm::models::log::LogSource::Relay,
            model_name: "gpt-4o".to_owned(),
            prompt_tokens: 10,
            completion_tokens: 5,
            quota: 42,
            is_stream: true,
            ..Default::default()
        },
    )
    .await
    .expect("insert log");
    let (items, total) = crate::llm::models::log::query_paged(
        &p,
        None,
        &crate::llm::models::log::LogFilters {
            token_id: Some(t.id.to_string()),
            ..Default::default()
        },
        1,
        10,
    )
    .await
    .expect("query");
    assert_eq!(total, 1);
    assert_eq!(items[0].model_name, "gpt-4o");
    assert_eq!(items[0].quota, 42);
}
