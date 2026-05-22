use serde::Deserialize;

#[derive(Debug, Deserialize, Default)]
pub struct WsQuery {
    pub filter: Option<String>,
}
