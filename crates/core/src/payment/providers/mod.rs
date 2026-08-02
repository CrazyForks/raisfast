#[cfg(feature = "payment-alipay")]
pub mod alipay;

#[cfg(feature = "payment-creem")]
pub mod creem;

#[cfg(feature = "payment-dodo")]
pub mod dodo;

#[cfg(feature = "payment-stripe")]
pub mod stripe;

#[cfg(feature = "payment-wechat")]
pub mod wechat;

use crate::errors::app_error::{AppError, AppResult};
use crate::payment::PaymentProvider;

#[cfg(any(
    feature = "payment-alipay",
    feature = "payment-creem",
    feature = "payment-dodo",
    feature = "payment-stripe",
    feature = "payment-wechat"
))]
mod shared {
    /// HTTP request timeout for payment gateway calls (seconds)
    const PAYMENT_TIMEOUT_SECS: u64 = 15;

    /// Build a reqwest client with payment-appropriate timeout
    pub(crate) fn http_client() -> reqwest::Client {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(PAYMENT_TIMEOUT_SECS))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new())
    }
}

#[cfg(any(
    feature = "payment-alipay",
    feature = "payment-creem",
    feature = "payment-dodo",
    feature = "payment-stripe",
    feature = "payment-wechat"
))]
pub(crate) use shared::http_client;

pub fn get_provider(
    provider_name: &str,
    encrypt_key: &[u8; 32],
) -> AppResult<Box<dyn PaymentProvider>> {
    let _ = encrypt_key;
    match provider_name {
        #[cfg(feature = "payment-alipay")]
        "alipay" => Ok(Box::new(alipay::AlipayProvider::new(*encrypt_key))),
        #[cfg(feature = "payment-creem")]
        "creem" => Ok(Box::new(creem::CreemProvider::new(*encrypt_key))),
        #[cfg(feature = "payment-dodo")]
        "dodo" => Ok(Box::new(dodo::DodoProvider::new(*encrypt_key))),
        #[cfg(feature = "payment-stripe")]
        "stripe" => Ok(Box::new(stripe::StripeProvider::new(*encrypt_key))),
        #[cfg(feature = "payment-wechat")]
        "wechat" => Ok(Box::new(wechat::WechatPayProvider::new(*encrypt_key))),
        _ => Err(AppError::BadRequest(format!(
            "unsupported payment provider: {provider_name}"
        ))),
    }
}
