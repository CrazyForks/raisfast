//! JSON bind coercion.
//!
//! The `crud_insert!` derive macro wraps values bound to `JSON`/`JSONB` columns
//! with [`DbJson::to_json`]. This converts `String` (and `Option` wrappers)
//! into the `serde_json::Value` that sqlx's `query!` macro requires for JSON
//! columns on PostgreSQL (Strong param checking). MySQL uses Weak checking so
//! String is accepted directly, but the conversion is harmless. SQLite stores
//! JSON as TEXT and its schema declares TEXT, so this coercion never triggers
//! for SQLite.
//!
//! `serde_json::Value` passes through unchanged.

/// Coerce a value bound to a JSON/JSONB column.
pub trait DbJson {
    /// The JSON-compatible type the value is coerced to.
    type Output;

    /// Convert into the JSON-compatible type.
    fn to_json(self) -> Self::Output;
}

impl DbJson for serde_json::Value {
    type Output = serde_json::Value;
    fn to_json(self) -> Self::Output {
        self
    }
}

impl DbJson for &serde_json::Value {
    type Output = serde_json::Value;
    fn to_json(self) -> Self::Output {
        self.clone()
    }
}

impl DbJson for String {
    type Output = serde_json::Value;
    fn to_json(self) -> Self::Output {
        serde_json::from_str(&self).unwrap_or(serde_json::Value::Null)
    }
}

impl DbJson for &String {
    type Output = serde_json::Value;
    fn to_json(self) -> Self::Output {
        serde_json::from_str(self).unwrap_or(serde_json::Value::Null)
    }
}

impl DbJson for &str {
    type Output = serde_json::Value;
    fn to_json(self) -> Self::Output {
        serde_json::from_str(self).unwrap_or(serde_json::Value::Null)
    }
}

impl DbJson for Option<String> {
    type Output = Option<serde_json::Value>;
    fn to_json(self) -> Self::Output {
        self.map(|s| serde_json::from_str(&s).unwrap_or(serde_json::Value::Null))
    }
}

impl DbJson for Option<&str> {
    type Output = Option<serde_json::Value>;
    fn to_json(self) -> Self::Output {
        self.map(|s| serde_json::from_str(s).unwrap_or(serde_json::Value::Null))
    }
}

impl DbJson for Option<serde_json::Value> {
    type Output = Option<serde_json::Value>;
    fn to_json(self) -> Self::Output {
        self
    }
}

impl DbJson for Option<&serde_json::Value> {
    type Output = Option<serde_json::Value>;
    fn to_json(self) -> Self::Output {
        self.cloned()
    }
}
