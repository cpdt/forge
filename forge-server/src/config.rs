use serde::Deserialize;
use std::collections::HashMap;
use std::net::SocketAddr;

#[derive(Deserialize, Debug)]
#[serde(rename_all = "kebab-case")]
pub struct Config {
    pub listen: SocketAddr,
    pub discord_token: String,
    pub discord_application: u64,

    pub servers: HashMap<String, ServerConfig>,

    pub maps: HashMap<String, String>,
    pub modes: HashMap<String, String>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "kebab-case")]
pub struct ServerConfig {
    pub channel: u64,
}
