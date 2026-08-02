use serde::Deserialize;

#[derive(Debug, Deserialize, Default)]
pub struct SubscribeQuery {
    pub filter: Option<String>,
}
