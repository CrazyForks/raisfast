use crate::errors::app_error::{AppError, AppResult};
use crate::models::payment_channel::PaymentChannel;

#[derive(Debug, Clone)]
pub struct RoutingContext {
    pub currency: String,
    pub country: Option<String>,
    pub language: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RankedChannel {
    pub channel: PaymentChannel,
    pub effective_priority: f64,
    pub is_recommended: bool,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct RoutingSettings {
    #[serde(default)]
    countries: Vec<String>,
    #[serde(default)]
    currencies: Vec<String>,
    #[serde(default)]
    languages: Vec<String>,
    #[serde(default)]
    priority: i64,
}

fn parse_settings(channel: &PaymentChannel) -> Option<RoutingSettings> {
    channel
        .settings
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
}

pub fn select_channels(channels: &[PaymentChannel], ctx: &RoutingContext) -> Vec<RankedChannel> {
    let mut ranked: Vec<RankedChannel> = channels
        .iter()
        .filter(|c| c.is_active != 0)
        .filter_map(|c| rank_channel(c, ctx))
        .collect();

    ranked.sort_by(|a, b| {
        b.effective_priority
            .partial_cmp(&a.effective_priority)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.channel.sort_order.cmp(&b.channel.sort_order))
    });

    if let Some(first) = ranked.first_mut() {
        first.is_recommended = true;
    }

    ranked
}

fn rank_channel(channel: &PaymentChannel, ctx: &RoutingContext) -> Option<RankedChannel> {
    let settings = parse_settings(channel);

    let base_priority = settings.as_ref().map(|s| s.priority).unwrap_or(0);

    let currencies = settings
        .as_ref()
        .map(|s| s.currencies.as_slice())
        .unwrap_or(&[]);

    let countries = settings
        .as_ref()
        .map(|s| s.countries.as_slice())
        .unwrap_or(&[]);

    let languages = settings
        .as_ref()
        .map(|s| s.languages.as_slice())
        .unwrap_or(&[]);

    if !currencies.is_empty()
        && !currencies
            .iter()
            .any(|c| c.eq_ignore_ascii_case(&ctx.currency))
    {
        return None;
    }

    let mut effective_priority = base_priority as f64;

    if let Some(ref country) = ctx.country {
        let upper = country.to_uppercase();
        if countries.iter().any(|c| c == "*") {
            effective_priority /= 10.0;
        } else if !countries.iter().any(|c| c.eq_ignore_ascii_case(&upper)) {
            return None;
        }
    } else if !countries.iter().any(|c| c == "*") {
        effective_priority /= 10.0;
    }

    if let Some(ref lang) = ctx.language {
        let lang_lower = lang.to_lowercase();
        let lang_prefix = lang_lower.split('-').next().unwrap_or(&lang_lower);

        let exact = languages
            .iter()
            .any(|l| l.eq_ignore_ascii_case(&lang_lower));
        let prefix = !exact
            && languages
                .iter()
                .any(|l| l.eq_ignore_ascii_case(lang_prefix));

        if exact {
            effective_priority += 50.0;
        } else if prefix {
            effective_priority += 25.0;
        }
    }

    Some(RankedChannel {
        channel: channel.clone(),
        effective_priority,
        is_recommended: false,
    })
}

pub fn select_best_channel(
    channels: &[PaymentChannel],
    ctx: &RoutingContext,
) -> AppResult<PaymentChannel> {
    let ranked = select_channels(channels, ctx);
    ranked.into_iter().next().map(|r| r.channel).ok_or_else(|| {
        AppError::BadRequest(format!(
            "no payment channel available for currency={}",
            ctx.currency
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::tz::Timestamp;

    fn make_channel(provider: &str, settings: Option<&str>) -> PaymentChannel {
        PaymentChannel {
            id: crate::types::snowflake_id::SnowflakeId(1),
            tenant_id: None,
            provider: provider.to_string(),
            name: provider.to_string(),
            is_live: 0,
            credentials: "{}".to_string(),
            webhook_secret: None,
            settings: settings.map(String::from),
            is_active: 1,
            sort_order: 0,
            version: 1,
            created_at: Timestamp::default(),
            updated_at: Timestamp::default(),
        }
    }

    fn make_channel_with_sort(provider: &str, settings: Option<&str>, sort: i64) -> PaymentChannel {
        PaymentChannel {
            sort_order: sort,
            ..make_channel(provider, settings)
        }
    }

    #[test]
    fn selects_by_currency() {
        let channels = vec![
            make_channel("stripe", Some(r#"{"currencies":["USD","EUR"]}"#)),
            make_channel("alipay", Some(r#"{"currencies":["CNY"]}"#)),
        ];
        let ctx = RoutingContext {
            currency: "CNY".into(),
            country: None,
            language: None,
        };
        let result = select_channels(&channels, &ctx);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].channel.provider, "alipay");
    }

    #[test]
    fn wildcard_country_demoted() {
        let channels = vec![
            make_channel("alipay", Some(r#"{"countries":["CN"],"priority":90}"#)),
            make_channel("dodo", Some(r#"{"countries":["*"],"priority":10}"#)),
        ];
        let ctx = RoutingContext {
            currency: "USD".into(),
            country: Some("CN".into()),
            language: None,
        };
        let result = select_channels(&channels, &ctx);
        assert_eq!(result[0].channel.provider, "alipay");
        assert_eq!(result[1].channel.provider, "dodo");
        assert!(result[0].effective_priority > result[1].effective_priority);
    }

    #[test]
    fn country_mismatch_excluded() {
        let channels = vec![
            make_channel("alipay", Some(r#"{"countries":["CN"]}"#)),
            make_channel("stripe", Some(r#"{"countries":["US","GB"]}"#)),
        ];
        let ctx = RoutingContext {
            currency: "USD".into(),
            country: Some("JP".into()),
            language: None,
        };
        let result = select_channels(&channels, &ctx);
        assert!(result.is_empty());
    }

    #[test]
    fn language_exact_match_bonus() {
        let channels = vec![
            make_channel("stripe", Some(r#"{"languages":["en"],"priority":50}"#)),
            make_channel("dodo", Some(r#"{"languages":["zh"],"priority":50}"#)),
        ];
        let ctx = RoutingContext {
            currency: "USD".into(),
            country: None,
            language: Some("zh-CN".into()),
        };
        let result = select_channels(&channels, &ctx);
        assert_eq!(result[0].channel.provider, "dodo");
    }

    #[test]
    fn language_prefix_match_bonus() {
        let channels = vec![
            make_channel("a", Some(r#"{"languages":["zh"],"priority":50}"#)),
            make_channel("b", Some(r#"{"priority":50}"#)),
        ];
        let ctx = RoutingContext {
            currency: "USD".into(),
            country: None,
            language: Some("zh-CN".into()),
        };
        let result = select_channels(&channels, &ctx);
        assert_eq!(result[0].channel.provider, "a");
    }

    #[test]
    fn first_result_marked_recommended() {
        let channels = vec![
            make_channel("alipay", Some(r#"{"priority":100}"#)),
            make_channel("stripe", Some(r#"{"priority":50}"#)),
        ];
        let ctx = RoutingContext {
            currency: "USD".into(),
            country: None,
            language: None,
        };
        let result = select_channels(&channels, &ctx);
        assert!(result[0].is_recommended);
        assert!(!result[1].is_recommended);
    }

    #[test]
    fn sort_order_tiebreak() {
        let channels = vec![
            make_channel_with_sort("stripe", Some(r#"{"priority":50}"#), 2),
            make_channel_with_sort("dodo", Some(r#"{"priority":50}"#), 1),
        ];
        let ctx = RoutingContext {
            currency: "USD".into(),
            country: None,
            language: None,
        };
        let result = select_channels(&channels, &ctx);
        assert_eq!(result[0].channel.provider, "dodo");
    }

    #[test]
    fn inactive_channel_excluded() {
        let mut ch = make_channel("stripe", Some(r#"{"priority":100}"#));
        ch.is_active = 0;
        let channels = vec![ch];
        let ctx = RoutingContext {
            currency: "USD".into(),
            country: None,
            language: None,
        };
        let result = select_channels(&channels, &ctx);
        assert!(result.is_empty());
    }

    #[test]
    fn no_settings_passes_through() {
        let channels = vec![make_channel("stripe", None)];
        let ctx = RoutingContext {
            currency: "USD".into(),
            country: None,
            language: None,
        };
        let result = select_channels(&channels, &ctx);
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn select_best_returns_error_when_empty() {
        let channels = vec![make_channel("stripe", Some(r#"{"currencies":["EUR"]}"#))];
        let ctx = RoutingContext {
            currency: "USD".into(),
            country: None,
            language: None,
        };
        assert!(select_best_channel(&channels, &ctx).is_err());
    }
}
