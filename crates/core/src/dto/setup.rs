use serde::{Deserialize, Serialize};
use validator::Validate;

#[derive(Debug, Serialize)]
pub struct SetupStatusResponse {
    pub database: DatabaseStatusInfo,
    pub storage: StorageStatusInfo,
    pub extensions: ExtensionsStatusInfo,
    pub has_admin: bool,
}

#[derive(Debug, Serialize)]
pub struct DatabaseStatusInfo {
    pub db_type: String,
    pub connected: bool,
    pub url_masked: String,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub database: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct StorageStatusInfo {
    pub writable: bool,
    pub path: String,
}

#[derive(Debug, Serialize)]
pub struct ExtensionsStatusInfo {
    pub writable: bool,
    pub content_types_path: String,
    pub plugins_path: String,
}

#[derive(Debug, Deserialize, Validate)]
pub struct SetupDatabaseRequest {
    pub host: Option<String>,
    pub port: Option<u16>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub database: Option<String>,
    #[validate(length(min = 1))]
    pub url: Option<String>,
}

impl SetupDatabaseRequest {
    pub fn build_url(&self, db_type: &str) -> Result<String, String> {
        if let Some(ref url) = self.url {
            return Ok(url.clone());
        }

        match db_type {
            "sqlite" => {
                let path = self
                    .database
                    .as_deref()
                    .unwrap_or("./storage/db/raisfast.db");
                Ok(format!("sqlite:{path}?mode=rwc"))
            }
            "postgres" => {
                let host = self.host.as_deref().ok_or("host is required")?;
                let port = self.port.unwrap_or(5432);
                let user = self.username.as_deref().ok_or("username is required")?;
                let pass = self.password.as_deref().unwrap_or("");
                let db = self.database.as_deref().ok_or("database is required")?;
                if pass.is_empty() {
                    Ok(format!("postgres://{user}@{host}:{port}/{db}"))
                } else {
                    Ok(format!("postgres://{user}:{pass}@{host}:{port}/{db}"))
                }
            }
            "mysql" => {
                let host = self.host.as_deref().ok_or("host is required")?;
                let port = self.port.unwrap_or(3306);
                let user = self.username.as_deref().ok_or("username is required")?;
                let pass = self.password.as_deref().unwrap_or("");
                let db = self.database.as_deref().ok_or("database is required")?;
                if pass.is_empty() {
                    Ok(format!("mysql://{user}@{host}:{port}/{db}"))
                } else {
                    Ok(format!("mysql://{user}:{pass}@{host}:{port}/{db}"))
                }
            }
            _ => Err(format!("unsupported db_type: {db_type}")),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct TestDatabaseResponse {
    pub connected: bool,
    pub db_type: String,
    pub url_masked: String,
    pub message: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SetupDatabaseResponse {
    pub restarting: bool,
}

#[derive(Debug, Deserialize, Validate)]
pub struct SetupInitRequest {
    #[validate(length(min = 2, max = 50))]
    pub username: String,
    #[validate(email)]
    pub email: String,
    #[validate(length(min = 8, max = 128))]
    pub password: String,
}
