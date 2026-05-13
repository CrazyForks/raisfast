#[cfg(feature = "payment-alipay")]
pub mod alipay;

#[cfg(feature = "payment-creem")]
pub mod creem;

#[cfg(feature = "payment-stripe")]
pub mod stripe;

#[cfg(feature = "payment-wechat")]
pub mod wechat;

use crate::errors::app_error::{AppError, AppResult};
use crate::payment::PaymentProvider;

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
        #[cfg(feature = "payment-stripe")]
        "stripe" => Ok(Box::new(stripe::StripeProvider::new(*encrypt_key))),
        #[cfg(feature = "payment-wechat")]
        "wechat" => Ok(Box::new(wechat::WechatPayProvider::new(*encrypt_key))),
        _ => Err(AppError::BadRequest(format!(
            "unsupported payment provider: {provider_name}"
        ))),
    }
}
