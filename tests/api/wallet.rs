use super::*;

static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn unique_tag() -> u64 {
    COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

async fn setup_admin() -> (axum::Router, AppState, String) {
    let (app, state) = test_app().await;
    raisfast::models::currencies::create(&state.pool, "default", "CNY", "Chinese Yuan", 2)
        .await
        .unwrap();
    raisfast::models::currencies::create(&state.pool, "default", "USD", "US Dollar", 2)
        .await
        .unwrap();
    let (int_id, id) = create_admin(&state.pool).await;
    let tok = make_token(&id, int_id, raisfast::models::user::UserRole::Admin);
    (app, state, tok)
}

async fn seed_user_get_id(app: &mut axum::Router, tag: u64) -> (String, String) {
    let email = format!("w{tag}@test.com");
    let username = format!("wuser{tag}");
    let (tok, _) = register_and_login(app, &email, &username, "Password123").await;
    let (_, body) = send(app, get_auth("/api/v1/users/me", &tok)).await;
    let id = body["data"]["id"].as_str().unwrap().to_string();
    (tok, id)
}

#[tokio::test]
async fn admin_credit_creates_wallet() {
    let tag = unique_tag();
    let (mut app, _state, tok) = setup_admin().await;
    let (_user_tok, user_id) = seed_user_get_id(&mut app, tag).await;

    let (status, body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/wallets/credit",
            json!({
                "user_id": user_id,
                "currency": "CNY",
                "transaction_no": format!("TXCREDIT{tag}"),
                "amount": 10000,
            }),
            &tok,
        ),
    )
    .await;
    assert!(status.is_success(), "credit: {status} {body:?}");
    assert_eq!(body["data"]["amount"], 10000);
    assert_eq!(body["data"]["entry_type"], "credit");
}

#[tokio::test]
async fn admin_debit_deducts_balance() {
    let tag = unique_tag();
    let (mut app, _state, tok) = setup_admin().await;
    let (_user_tok, user_id) = seed_user_get_id(&mut app, tag).await;

    let _ = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/wallets/credit",
            json!({
                "user_id": user_id,
                "currency": "CNY",
                "transaction_no": format!("TXPRE{tag}"),
                "amount": 5000,
            }),
            &tok,
        ),
    )
    .await;

    let (status, body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/wallets/debit",
            json!({
                "user_id": user_id,
                "currency": "CNY",
                "transaction_no": format!("TXDEBIT{tag}"),
                "amount": 2000,
            }),
            &tok,
        ),
    )
    .await;
    assert!(status.is_success(), "debit: {status} {body:?}");
    assert_eq!(body["data"]["amount"], 2000);
    assert_eq!(body["data"]["entry_type"], "debit");
}

#[tokio::test]
async fn user_list_wallets_after_credit() {
    let tag = unique_tag();
    let (mut app, _state, tok) = setup_admin().await;
    let (user_tok, user_id) = seed_user_get_id(&mut app, tag).await;

    let _ = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/wallets/credit",
            json!({
                "user_id": user_id,
                "currency": "CNY",
                "transaction_no": format!("TXLIST{tag}"),
                "amount": 3000,
            }),
            &tok,
        ),
    )
    .await;

    let (status, body) = send(&mut app, get_auth("/api/v1/wallets", &user_tok)).await;
    assert!(status.is_success(), "list wallets: {status} {body:?}");
    let items = body["data"].as_array().unwrap();
    assert!(!items.is_empty());
    let cny = items.iter().find(|w| w["currency"] == "CNY").unwrap();
    assert_eq!(cny["balance"], 3000);
}

#[tokio::test]
async fn user_get_wallet_by_currency() {
    let tag = unique_tag();
    let (mut app, _state, tok) = setup_admin().await;
    let (user_tok, user_id) = seed_user_get_id(&mut app, tag).await;

    let _ = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/wallets/credit",
            json!({
                "user_id": user_id,
                "currency": "USD",
                "transaction_no": format!("TXUSD{tag}"),
                "amount": 5000,
            }),
            &tok,
        ),
    )
    .await;

    let (status, body) = send(&mut app, get_auth("/api/v1/wallets/USD", &user_tok)).await;
    assert!(status.is_success(), "get wallet: {status} {body:?}");
    assert_eq!(body["data"]["currency"], "USD");
    assert_eq!(body["data"]["balance"], 5000);
}

#[tokio::test]
async fn admin_list_all_wallets() {
    let tag = unique_tag();
    let (mut app, _state, tok) = setup_admin().await;
    let (_user_tok, user_id) = seed_user_get_id(&mut app, tag).await;

    let _ = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/wallets/credit",
            json!({
                "user_id": user_id,
                "currency": "CNY",
                "transaction_no": format!("TXALIST{tag}"),
                "amount": 1000,
            }),
            &tok,
        ),
    )
    .await;

    let (status, body) = send(
        &mut app,
        get_auth("/api/v1/admin/wallets?page=1&page_size=10", &tok),
    )
    .await;
    assert!(status.is_success(), "admin list wallets: {status} {body:?}");
    let items = body["data"]["items"].as_array().unwrap();
    assert!(!items.is_empty());
}

#[tokio::test]
async fn admin_reversal_restores_balance() {
    let tag = unique_tag();
    let (mut app, _state, tok) = setup_admin().await;
    let (_user_tok, user_id) = seed_user_get_id(&mut app, tag).await;

    let (_, credit_body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/wallets/credit",
            json!({
                "user_id": user_id,
                "currency": "CNY",
                "transaction_no": format!("TXREVPRE{tag}"),
                "amount": 5000,
            }),
            &tok,
        ),
    )
    .await;

    let tx_id = credit_body["data"]["id"].as_str().unwrap();

    let (status, body) = send(
        &mut app,
        post_json_auth(
            &format!("/api/v1/admin/wallets/{tx_id}/reversal"),
            json!({"transaction_no": format!("TXREV{tag}")}),
            &tok,
        ),
    )
    .await;
    assert!(status.is_success(), "reversal: {status} {body:?}");
    assert_eq!(body["data"]["entry_type"], "debit");
}

#[tokio::test]
async fn user_list_transactions() {
    let tag = unique_tag();
    let (mut app, _state, tok) = setup_admin().await;
    let (user_tok, user_id) = seed_user_get_id(&mut app, tag).await;

    let _ = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/wallets/credit",
            json!({
                "user_id": user_id,
                "currency": "CNY",
                "transaction_no": format!("TXTXN{tag}"),
                "amount": 1000,
            }),
            &tok,
        ),
    )
    .await;

    let (status, body) = send(
        &mut app,
        get_auth(
            "/api/v1/wallets/transactions?page=1&page_size=10",
            &user_tok,
        ),
    )
    .await;
    assert!(status.is_success(), "list txns: {status} {body:?}");
    let items = body["data"]["items"].as_array().unwrap();
    assert!(!items.is_empty());
}

#[tokio::test]
async fn debit_insufficient_balance_rejected() {
    let tag = unique_tag();
    let (mut app, _state, tok) = setup_admin().await;
    let (_user_tok, user_id) = seed_user_get_id(&mut app, tag).await;

    let (status, _) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/wallets/debit",
            json!({
                "user_id": user_id,
                "currency": "CNY",
                "transaction_no": format!("TXINSUFF{tag}"),
                "amount": 99999,
            }),
            &tok,
        ),
    )
    .await;
    assert!(!status.is_success(), "should reject insufficient balance");
}
