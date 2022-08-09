use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Config {
    pub discord_token: String,
    pub discord_application: u64,

    pub servers: HashMap<String, ServerConfig>,

    pub maps: HashMap<String, String>,
    pub modes: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct ServerConfig {
    pub address: String,
    pub password: String,
    pub channel: u64,
}
