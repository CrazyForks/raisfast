use serde::{Deserialize, Deserializer, Serialize, Serializer};
use utoipa::ToSchema;

/// LLM billing unit stored internally as integer **micro-USD** quota.
///
/// Mirrors the `Price` / `SnowflakeId` pattern: the wire format differs from
/// storage (see `dev-docs/llm/pricing.md` §4).
/// - DB / arithmetic: integer `i64` quota (`$1 = QUOTA_PER_USD`), no float drift;
///   balance deduction relies on atomic integer `WHERE remain >= X` comparisons.
/// - `Serialize`: emits **USD** (quota ÷ 1M) as a JSON number, so admin/bill
///   clients receive display-ready amounts.
/// - `Deserialize`: accepts **USD** (number) × 1M → quota.
///
/// `QUOTA_PER_USD` is the data format itself (like `Price`'s ×100): hardcoded,
/// never configurable — changing it would silently re-value every stored row.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, sqlx::Type,
)]
#[sqlx(transparent)]
pub struct Quota(pub i64);

/// $1 = 1,000,000 quota (micro-USD). Constant — see type docs.
pub const QUOTA_PER_USD: f64 = 1_000_000.0;

impl Quota {
    /// Construct from integer quota (the internal unit).
    pub fn from_quota(v: i64) -> Self {
        Quota(v)
    }

    /// Construct from a USD amount (e.g. `Quota::from_usd(11.25)`).
    pub fn from_usd(v: f64) -> Self {
        Quota((v * QUOTA_PER_USD).round() as i64)
    }

    /// Construct from a USD amount, rounding **up** to the next quota.
    /// Billing uses this: never under-charge, error is < 1 quota (< $0.000001).
    ///
    /// f64 representation error can push an exact integer boundary a hair
    /// above it (e.g. `0.0071 × 0.8 × 1e6 == 5680.000000000001`); naive ceil
    /// would add a phantom quota. Values within `EPS` quota of an integer are
    /// snapped to it — `EPS` (1e-6) is a trillionth of a cent, far below any
    /// real fractional charge and far above f64 error at these magnitudes.
    pub fn from_usd_ceil(v: f64) -> Self {
        const EPS: f64 = 1e-6;
        let x = v * QUOTA_PER_USD;
        let rounded = x.round();
        let n = if (x - rounded).abs() < EPS {
            rounded
        } else {
            x.ceil()
        };
        Quota(n as i64)
    }

    /// The amount in USD as a float (display use only).
    pub fn as_usd(&self) -> f64 {
        self.0 as f64 / QUOTA_PER_USD
    }
}

impl From<i64> for Quota {
    fn from(v: i64) -> Self {
        Quota(v)
    }
}

impl From<Quota> for i64 {
    fn from(v: Quota) -> Self {
        v.0
    }
}

impl std::fmt::Display for Quota {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:.6}", self.as_usd())
    }
}

impl std::ops::Add for Quota {
    type Output = Quota;
    fn add(self, rhs: Quota) -> Quota {
        Quota(self.0 + rhs.0)
    }
}

impl std::ops::Sub for Quota {
    type Output = Quota;
    fn sub(self, rhs: Quota) -> Quota {
        Quota(self.0 - rhs.0)
    }
}

impl std::ops::AddAssign for Quota {
    fn add_assign(&mut self, rhs: Quota) {
        self.0 += rhs.0;
    }
}

impl std::ops::SubAssign for Quota {
    fn sub_assign(&mut self, rhs: Quota) {
        self.0 -= rhs.0;
    }
}

impl Serialize for Quota {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.as_usd().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Quota {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let usd = f64::deserialize(deserializer)?;
        Ok(Quota::from_usd(usd))
    }
}

impl utoipa::PartialSchema for Quota {
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
        utoipa::openapi::ObjectBuilder::new()
            .schema_type(utoipa::openapi::schema::Type::Number)
            .format(Some(utoipa::openapi::SchemaFormat::KnownFormat(
                utoipa::openapi::KnownFormat::Double,
            )))
            .into()
    }
}

impl ToSchema for Quota {
    fn name() -> std::borrow::Cow<'static, str> {
        std::borrow::Cow::Borrowed("number")
    }
}

#[cfg(feature = "export-types")]
impl ts_rs::TS for Quota {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usd_roundtrip() {
        let q = Quota::from_usd(11.25);
        assert_eq!(q.0, 11_250_000);
        assert!((q.as_usd() - 11.25).abs() < 1e-9);
    }

    #[test]
    fn tiny_amounts_stay_exact() {
        // A single embedding request bills $0.000375 → 375 quota, no drift.
        let q = Quota::from_usd(0.000375);
        assert_eq!(q.0, 375);
    }

    #[test]
    fn serde_emits_usd_number() {
        let json = serde_json::to_string(&Quota::from_usd(2.5)).unwrap_or_default();
        assert_eq!(json, "2.5");
    }

    #[test]
    fn ceil_snaps_float_boundary_no_phantom_quota() {
        // 0.0071 × 0.8 × 1e6 computes to 5680.000000000001 in f64 — naive
        // ceil would yield 5681; the epsilon snap keeps it at 5680.
        assert_eq!(Quota::from_usd_ceil(0.0071 * 0.8), Quota(5_680));
        // Genuine fractions still round up.
        assert_eq!(Quota::from_usd_ceil(0.000333), Quota(333));
        assert_eq!(Quota::from_usd_ceil(0.0003331), Quota(334));
    }
}
