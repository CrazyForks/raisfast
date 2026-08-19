use serde::{Deserialize, Deserializer, Serialize, Serializer};
use utoipa::ToSchema;

/// Money stored internally as integer **cents** (smallest currency unit).
///
/// Mirrors the `SnowflakeId` pattern: the wire format differs from storage.
/// - DB / arithmetic: integer `i64` cents (no float errors), `#[sqlx(transparent)]`
/// - `Serialize`: emits **yuan** (cents ÷ 100) as a JSON number, so storefront
///   clients receive display-ready amounts (Shopify-style).
/// - `Deserialize`: accepts **yuan** (number or string), ×100 → cents.
///
/// The API contract is "the price a customer sees", never the internal unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, sqlx::Type)]
#[sqlx(transparent)]
pub struct Price(pub i64);

impl Price {
    /// Construct from integer cents (the internal unit).
    pub fn from_cents(v: i64) -> Self {
        Price(v)
    }

    /// Construct from yuan (e.g. `Price::from_yuan(19.99)`).
    pub fn from_yuan(v: f64) -> Self {
        Price((v * 100.0).round() as i64)
    }

    /// The amount in yuan as a float (display use only).
    pub fn as_yuan(&self) -> f64 {
        self.0 as f64 / 100.0
    }
}

impl From<i64> for Price {
    fn from(v: i64) -> Self {
        Price(v)
    }
}

impl From<Price> for i64 {
    fn from(v: Price) -> Self {
        v.0
    }
}

impl std::fmt::Display for Price {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:.2}", self.as_yuan())
    }
}

impl std::ops::Add for Price {
    type Output = Price;
    fn add(self, rhs: Price) -> Price {
        Price(self.0 + rhs.0)
    }
}

impl std::ops::AddAssign for Price {
    fn add_assign(&mut self, rhs: Price) {
        self.0 += rhs.0;
    }
}

impl std::ops::Sub for Price {
    type Output = Price;
    fn sub(self, rhs: Price) -> Price {
        Price(self.0 - rhs.0)
    }
}

impl std::ops::Mul<i64> for Price {
    type Output = Price;
    fn mul(self, rhs: i64) -> Price {
        Price(self.0 * rhs)
    }
}

impl Price {
    /// Saturating multiply by a quantity (never panics on overflow).
    pub fn checked_mul_qty(self, qty: i64) -> Option<Price> {
        self.0.checked_mul(qty).map(Price)
    }

    /// Saturating add (never panics on overflow).
    pub fn checked_add_price(self, other: Price) -> Option<Price> {
        self.0.checked_add(other.0).map(Price)
    }

    /// Maximum of two amounts.
    pub fn max_price(self, other: Price) -> Price {
        Price(self.0.max(other.0))
    }

    /// Minimum of two amounts.
    pub fn min_price(self, other: Price) -> Price {
        Price(self.0.min(other.0))
    }
}

impl std::iter::Sum for Price {
    fn sum<I: Iterator<Item = Price>>(iter: I) -> Price {
        iter.fold(Price(0), |acc, p| acc + p)
    }
}

impl Serialize for Price {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        // Whole-yuan amounts serialize as integer JSON numbers (e.g. `198`,
        // not `198.0`) — canonical JSON; fractional amounts keep decimal form
        // (e.g. `198.5`). Both parse identically as JS numbers.
        if self.0 % 100 == 0 {
            serializer.serialize_i64(self.0 / 100)
        } else {
            serializer.serialize_f64(self.as_yuan())
        }
    }
}

impl<'de> Deserialize<'de> for Price {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct PriceVisitor;
        impl<'de> serde::de::Visitor<'de> for PriceVisitor {
            type Value = Price;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a number or string representing an amount in yuan")
            }
            fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<Price, E> {
                Ok(Price::from_yuan(v))
            }
            fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Price, E> {
                Ok(Price::from_yuan(v as f64))
            }
            fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Price, E> {
                Ok(Price::from_yuan(v as f64))
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Price, E> {
                v.parse::<f64>()
                    .map(Price::from_yuan)
                    .map_err(|_| E::custom("invalid amount"))
            }
        }
        deserializer.deserialize_any(PriceVisitor)
    }
}

impl utoipa::PartialSchema for Price {
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
        utoipa::openapi::ObjectBuilder::new()
            .schema_type(utoipa::openapi::schema::Type::Number)
            .format(Some(utoipa::openapi::SchemaFormat::KnownFormat(
                utoipa::openapi::KnownFormat::Double,
            )))
            .into()
    }
}

impl ToSchema for Price {
    fn name() -> std::borrow::Cow<'static, str> {
        std::borrow::Cow::Borrowed("number")
    }
}

#[cfg(feature = "export-types")]
impl ts_rs::TS for Price {
    type WithoutGenerics = Self;
    type OptionInnerType = Self;
    fn name(_: &ts_rs::Config) -> String {
        "number".into()
    }
    fn inline(_: &ts_rs::Config) -> String {
        "number".into()
    }
    fn decl(_: &ts_rs::Config) -> String {
        String::new()
    }
    fn decl_concrete(_: &ts_rs::Config) -> String {
        String::new()
    }
}
