//! DB-backed tests for the llm module: relay auth chain, quota billing and
//! key-state persistence — each test gets its own SQLite in-memory schema
//! via `crate::test_pool!()`.

use crate::commands::CreateUserCmd;
use crate::llm::billing::QUOTA_PER_CENT;
use crate::llm::models::token::{self, LlmTokenStatus};
use crate::llm::relay::auth;
use crate::models::user::{self, UserStatus};
use crate::types::price::Price;
use crate::types::quota::Quota;
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
        &plain,
        Quota(remain),
        unlimited,
        None,
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
async fn auth_cache_invalidated_on_status_write() {
    let p = pool().await;
    let uid = make_user(&p, UserStatus::Active).await;
    let (plain, t) = make_token(&p, uid, 100, false).await;

    // Warm the short-TTL cache.
    assert!(auth::authenticate(&p, &plain, "").await.is_ok());
    // A status write must evict immediately (write-path invalidation, §9.1) —
    // no waiting out the 5s TTL.
    token::update_status(&p, None, t.id, LlmTokenStatus::Disabled)
        .await
        .unwrap();
    assert!(
        auth::authenticate(&p, &plain, "").await.is_err(),
        "disable is visible immediately despite the warm cache"
    );
    token::update_status(&p, None, t.id, LlmTokenStatus::Enabled)
        .await
        .unwrap();
    assert!(
        auth::authenticate(&p, &plain, "").await.is_ok(),
        "re-enable is visible immediately"
    );
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
        &plain,
        Quota(100),
        false,
        None,
        None,
        Some("10.1.0.0/16\n127.0.0.1"),
        None,
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
    let charge = crate::llm::relay::billing::pre_consume(&p, &t, "default", Quota(300))
        .await
        .expect("hold");
    assert_eq!(charge.pre_consumed, Quota(300));
    let mid = token::find_by_id(&p, t.id, None).await.unwrap().unwrap();
    assert_eq!(mid.remain_quota, Quota(700));

    // Actual 100 → refund 200 → remain 900, used tracks the actual 100.
    crate::llm::relay::billing::settle(&p, &charge, Quota(100)).await;
    let after = token::find_by_id(&p, t.id, None).await.unwrap().unwrap();
    assert_eq!(after.remain_quota, Quota(900));
    assert_eq!(after.used_quota, Quota(100));

    // Full refund restores to the original hold.
    let charge2 = crate::llm::relay::billing::pre_consume(&p, &t, "default", Quota(250))
        .await
        .expect("hold2");
    crate::llm::relay::billing::refund_all(&p, &charge2).await;
    let final_row = token::find_by_id(&p, t.id, None).await.unwrap().unwrap();
    assert_eq!(final_row.remain_quota, Quota(900));
}

#[tokio::test]
async fn billing_preconsume_rejects_insufficient() {
    let p = pool().await;
    let uid = make_user(&p, UserStatus::Active).await;
    let (_plain, t) = make_token(&p, uid, 50, false).await;
    let err = crate::llm::relay::billing::pre_consume(&p, &t, "default", Quota(300))
        .await
        .expect_err("must reject");
    assert!(matches!(
        err,
        crate::errors::app_error::AppError::TooManyRequests(_)
    ));
    // Overdraw protection: unchanged balance.
    let row = token::find_by_id(&p, t.id, None).await.unwrap().unwrap();
    assert_eq!(row.remain_quota, Quota(50));
}

#[tokio::test]
async fn billing_under_hold_top_up_collects_or_logs() {
    let p = pool().await;
    let uid = make_user(&p, UserStatus::Active).await;
    let (_plain, t) = make_token(&p, uid, 500, false).await;
    let charge = crate::llm::relay::billing::pre_consume(&p, &t, "default", Quota(100))
        .await
        .expect("hold");
    // Actual 150 > hold 100 → collects the 50 diff.
    crate::llm::relay::billing::settle(&p, &charge, Quota(150)).await;
    let row = token::find_by_id(&p, t.id, None).await.unwrap().unwrap();
    assert_eq!(row.remain_quota, Quota(350));
}

#[tokio::test]
async fn billing_unlimited_token_skips_holds() {
    let p = pool().await;
    let uid = make_user(&p, UserStatus::Active).await;
    let (_plain, t) = make_token(&p, uid, 0, true).await;
    let charge = crate::llm::relay::billing::pre_consume(&p, &t, "default", Quota(12345))
        .await
        .expect("no hold for unlimited");
    crate::llm::relay::billing::settle(&p, &charge, Quota(999)).await;
    crate::llm::relay::billing::refund_all(&p, &charge).await;
    let row = token::find_by_id(&p, t.id, None).await.unwrap().unwrap();
    assert_eq!(
        row.remain_quota,
        Quota(0),
        "unlimited never touches remain_quota"
    );
}

/// Seed a metered billing policy for an isolated tenant (options fall back
/// tenant → global, so a unique tenant keeps shared-DB runs leak-free) plus
/// its active CNY currency row.
async fn seed_metered(p: &crate::db::Pool, tenant: &str) {
    crate::models::options::upsert_value(
        p,
        "llm.billing.mode",
        &serde_json::json!("metered"),
        Some(tenant),
    )
    .await
    .expect("option");
    let ph = crate::db::Driver::ph;
    use crate::db::driver::DbDriver;
    let sql = format!(
        "INSERT INTO currencies (id, tenant_id, code, name) VALUES ({}, {}, {}, {})",
        ph(1),
        ph(2),
        ph(3),
        ph(4)
    );
    sqlx::query(crate::db::safe_sql(&sql))
        .bind(crate::utils::id::new_id())
        .bind(tenant)
        .bind("CNY")
        .bind("Chinese Yuan")
        .execute(p)
        .await
        .expect("currency");
}

#[tokio::test]
async fn billing_metered_wallet_holds_and_settles() {
    let p = pool().await;
    let uid = make_user(&p, UserStatus::Active).await;
    let (_plain, t) = make_token(&p, uid, 1_000_000, false).await;
    let tenant = format!("metered-{}", crate::utils::id::new_id());
    seed_metered(&p, &tenant).await;
    // Fund $10 → 1000¢ via the settle primitive's credit path.
    crate::services::wallet::llm_settle(
        &p,
        Some(&tenant),
        uid,
        "CNY",
        Price(1_000),
        0,
        QUOTA_PER_CENT,
        "seed-metered-holds",
        None,
    )
    .await
    .expect("seed credit");

    // 500K quota = $0.5 = 50¢ hold; token 1M → 500K remain.
    let charge = crate::llm::relay::billing::pre_consume(&p, &t, &tenant, Quota(500_000))
        .await
        .expect("hold");
    let w = crate::models::wallet::find_by_user_and_currency(&p, uid, "CNY")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(w.balance, Price(950));
    let row = token::find_by_id(&p, t.id, None).await.unwrap().unwrap();
    assert_eq!(row.remain_quota, Quota(500_000));

    // Actual 200K quota = 20¢ → wallet refund 30¢, token refund 300K.
    crate::llm::relay::billing::settle(&p, &charge, Quota(200_000)).await;
    let w = crate::models::wallet::find_by_user_and_currency(&p, uid, "CNY")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(w.balance, Price(980));
    let row = token::find_by_id(&p, t.id, None).await.unwrap().unwrap();
    assert_eq!(row.remain_quota, Quota(800_000));
    assert_eq!(row.used_quota, Quota(200_000));

    // Full refund restores both ledgers.
    let charge2 = crate::llm::relay::billing::pre_consume(&p, &t, &tenant, Quota(100_000))
        .await
        .expect("hold2");
    crate::llm::relay::billing::refund_all(&p, &charge2).await;
    let w = crate::models::wallet::find_by_user_and_currency(&p, uid, "CNY")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(w.balance, Price(980));
    let row = token::find_by_id(&p, t.id, None).await.unwrap().unwrap();
    assert_eq!(row.remain_quota, Quota(800_000));
}

#[tokio::test]
async fn billing_metered_empty_wallet_rejects_both_ledgers_untouched() {
    let p = pool().await;
    let uid = make_user(&p, UserStatus::Active).await;
    let (_plain, t) = make_token(&p, uid, 1_000_000, false).await;
    let tenant = format!("metered-{}", crate::utils::id::new_id());
    seed_metered(&p, &tenant).await;

    let err = crate::llm::relay::billing::pre_consume(&p, &t, &tenant, Quota(500_000))
        .await
        .expect_err("empty wallet must reject");
    assert!(matches!(
        err,
        crate::errors::app_error::AppError::TooManyRequests(_)
    ));
    // Token debit unwound; the wallet tx rolled back (no row persisted).
    let row = token::find_by_id(&p, t.id, None).await.unwrap().unwrap();
    assert_eq!(row.remain_quota, Quota(1_000_000));
    assert!(
        crate::models::wallet::find_by_user_and_currency(&p, uid, "CNY")
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn billing_free_mode_never_touches_wallet() {
    let p = pool().await;
    let uid = make_user(&p, UserStatus::Active).await;
    let (_plain, t) = make_token(&p, uid, 1_000_000, false).await;
    let charge = crate::llm::relay::billing::pre_consume(&p, &t, "default", Quota(500_000))
        .await
        .expect("hold");
    assert!(charge.wallet.is_none(), "free mode carries no wallet hold");
    assert!(
        crate::models::wallet::find_by_user_and_currency(&p, uid, "CNY")
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn billing_metered_unlimited_token_charges_wallet_only() {
    let p = pool().await;
    let uid = make_user(&p, UserStatus::Active).await;
    let (_plain, t) = make_token(&p, uid, 0, true).await;
    let tenant = format!("metered-{}", crate::utils::id::new_id());
    seed_metered(&p, &tenant).await;
    crate::services::wallet::llm_settle(
        &p,
        Some(&tenant),
        uid,
        "CNY",
        Price(1_000),
        0,
        QUOTA_PER_CENT,
        "seed-metered-unlimited",
        None,
    )
    .await
    .expect("seed credit");

    let charge = crate::llm::relay::billing::pre_consume(&p, &t, &tenant, Quota(500_000))
        .await
        .expect("wallet holds even for unlimited tokens");
    assert!(charge.unlimited);
    let w = crate::models::wallet::find_by_user_and_currency(&p, uid, "CNY")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(w.balance, Price(950), "wallet is the real gate");
    crate::llm::relay::billing::settle(&p, &charge, Quota(200_000)).await;
    let w = crate::models::wallet::find_by_user_and_currency(&p, uid, "CNY")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(w.balance, Price(980));
    let row = token::find_by_id(&p, t.id, None).await.unwrap().unwrap();
    assert_eq!(row.remain_quota, Quota(0), "token ledger untouched");
}

/// Sub-cent usage accumulates in the wallet carry; the wallet is debited a
/// whole cent only when the carry crosses QUOTA_PER_CENT — net wallet spend
/// converges to the exact llm meter total within one cent.
#[tokio::test]
async fn billing_metered_carry_accumulates_sub_cent_usage() {
    let p = pool().await;
    let uid = make_user(&p, UserStatus::Active).await;
    let (_plain, t) = make_token(&p, uid, 1_000_000, false).await;
    let tenant = format!("metered-{}", crate::utils::id::new_id());
    seed_metered(&p, &tenant).await;
    crate::services::wallet::llm_settle(
        &p,
        Some(&tenant),
        uid,
        "CNY",
        Price(1_000),
        0,
        QUOTA_PER_CENT,
        "seed-metered-carry",
        None,
    )
    .await
    .expect("seed credit");

    // Two calls, each holding 1¢ and consuming 6_000 quota ($0.006) —
    // sub-cent on its own.
    let charge1 = crate::llm::relay::billing::pre_consume(&p, &t, &tenant, Quota(10_000))
        .await
        .expect("hold1");
    let w = crate::models::wallet::find_by_user_and_currency(&p, uid, "CNY")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(w.balance, Price(999), "1¢ held");

    // Usage 6_000 < 1 cent → charge 0, full hold refunded, carry = 6_000.
    crate::llm::relay::billing::settle(&p, &charge1, Quota(6_000)).await;
    let w = crate::models::wallet::find_by_user_and_currency(&p, uid, "CNY")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(w.balance, Price(1_000), "refunded, sub-cent carried");
    assert_eq!(w.llm_carry_quota, 6_000);

    // Second call: pending 12_000 → charge 1¢, carry 2_000, diff 0 (no rows).
    let charge2 = crate::llm::relay::billing::pre_consume(&p, &t, &tenant, Quota(10_000))
        .await
        .expect("hold2");
    crate::llm::relay::billing::settle(&p, &charge2, Quota(6_000)).await;
    let w = crate::models::wallet::find_by_user_and_currency(&p, uid, "CNY")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(w.balance, Price(999), "carry crossed the line: 1¢ charged");
    assert_eq!(w.llm_carry_quota, 2_000);
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
            cost_mode: crate::llm::models::channel::LlmCostMode::Usage,
            cost_discount: 1.0,
            monthly_cost: None,
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
            quota: Quota(42),
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
    assert_eq!(items[0].quota, Quota(42));
    // Day defaults to today (UTC) when not provided.
    let today = crate::utils::tz::now_utc().format("%Y-%m-%d").to_string();
    assert_eq!(items[0].day, today);
}

#[tokio::test]
async fn log_daily_stats_aggregates_by_day() {
    use crate::llm::models::log::{self, LogSource, NewLog};
    let p = pool().await;
    let uid = make_user(&p, UserStatus::Active).await;
    let today = crate::utils::tz::now_utc();
    let d0 = today.format("%Y-%m-%d").to_string();
    let d1 = (today - chrono::Duration::days(1))
        .format("%Y-%m-%d")
        .to_string();

    let mk = |day: &str, prompt: i32, completion: i32, quota: i64, cost: i64| NewLog {
        tenant_id: Some("default".to_owned()),
        user_id: Some(uid),
        source: LogSource::Relay,
        model_name: "m".to_owned(),
        prompt_tokens: prompt,
        completion_tokens: completion,
        quota: Quota(quota),
        cost_quota: Quota(cost),
        day: Some(day.to_owned()),
        ..Default::default()
    };
    // Two rows today, one yesterday.
    log::insert_log(&p, mk(&d0, 100, 10, 300, 120))
        .await
        .unwrap();
    log::insert_log(&p, mk(&d0, 200, 20, 500, 200))
        .await
        .unwrap();
    log::insert_log(&p, mk(&d1, 50, 5, 100, 40)).await.unwrap();

    let stats = log::daily_stats(&p, None, 2).await.unwrap();
    assert_eq!(stats.len(), 2, "one bucket per day with rows");
    let today_row = stats.iter().find(|s| s.date == d0).unwrap();
    assert_eq!(today_row.requests, 2);
    assert_eq!(today_row.prompt_tokens, 300);
    assert_eq!(today_row.completion_tokens, 30);
    assert_eq!(today_row.quota, 800);
    assert_eq!(today_row.cost_quota, 320);
    let yesterday = stats.iter().find(|s| s.date == d1).unwrap();
    assert_eq!(yesterday.requests, 1);
    assert_eq!(yesterday.quota, 100);

    // Window excludes the older row when days=1 (only today).
    let stats1 = log::daily_stats(&p, None, 1).await.unwrap();
    assert_eq!(stats1.len(), 1);
    assert_eq!(stats1[0].date, d0);
}

#[tokio::test]
async fn archive_rollup_is_idempotent_and_stats_merge_summary() {
    use crate::llm::models::log::{self, LogSource, NewLog};
    let p = pool().await;
    let old = "2020-01-01";
    let old2 = "2020-01-02";
    let mk = |source: LogSource, day: &str, quota: i64| NewLog {
        tenant_id: Some("default".to_owned()),
        source,
        model_name: "gpt-4o".to_owned(),
        quota: Quota(quota),
        prompt_tokens: 10,
        completion_tokens: 5,
        day: Some(day.to_owned()),
        ..Default::default()
    };
    log::insert_log(&p, mk(LogSource::Relay, old, 100))
        .await
        .unwrap();
    log::insert_log(&p, mk(LogSource::Relay, old2, 50))
        .await
        .unwrap();
    log::insert_log(&p, mk(LogSource::Test, old, 7))
        .await
        .unwrap();

    // Both default (90) and test (7) windows are far past → all 3 archived.
    let r1 = log::archive_old_logs(&p, 90, 7).await.unwrap();
    assert_eq!(r1.deleted_rows, 3);
    assert!(r1.summary_rows >= 2, "relay + test grains: {r1:?}");

    // Re-run: detail already gone → no rows touched, no double count.
    let r2 = log::archive_old_logs(&p, 90, 7).await.unwrap();
    assert_eq!(r2.deleted_rows, 0);
    assert_eq!(r2.summary_rows, 0);

    // Long-term report now reads the summary: day buckets merge all sources.
    let buckets = log::stats_by(&p, Some("default"), "day", "2019-12-01", "2020-12-31")
        .await
        .unwrap();
    let d1 = buckets.iter().find(|b| b.key == old).expect("day old");
    assert_eq!(d1.requests, 2, "relay + test rows merged");
    assert_eq!(d1.quota, 107);
    let d2 = buckets.iter().find(|b| b.key == old2).expect("day old2");
    assert_eq!(d2.requests, 1);
    assert_eq!(d2.quota, 50);

    // Detail rows are gone.
    let (_items, total) = log::query_paged(
        &p,
        None,
        &log::LogFilters {
            model_name: Some("gpt-4o".to_owned()),
            ..Default::default()
        },
        1,
        10,
    )
    .await
    .unwrap();
    assert_eq!(total, 0, "old detail rows deleted");
}

#[tokio::test]
async fn usage_by_user_reads_archived_summary_only_for_that_user() {
    use crate::llm::models::log::{self, LogSource, NewLog};
    let p = pool().await;
    let u1 = make_user(&p, UserStatus::Active).await;
    let u2 = make_user(&p, UserStatus::Active).await;
    for (uid, quota) in [(u1, 100i64), (u2, 900i64)] {
        log::insert_log(
            &p,
            NewLog {
                tenant_id: Some("default".to_owned()),
                user_id: Some(uid),
                source: LogSource::Relay,
                model_name: "gpt-4o".to_owned(),
                quota: Quota(quota),
                day: Some("2020-01-01".to_owned()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    }
    log::archive_old_logs(&p, 90, 7).await.unwrap();

    let buckets = log::usage_by_user(&p, None, u1, "model", "2019-01-01", "2020-12-31")
        .await
        .unwrap();
    assert_eq!(buckets.len(), 1);
    assert_eq!(buckets[0].quota, 100, "only the caller's archived usage");
    assert_eq!(buckets[0].requests, 1);
}
