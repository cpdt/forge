use serde::Deserialize;
use std::net::SocketAddr;

#[derive(Deserialize, Debug)]
#[serde(rename_all = "kebab-case")]
pub struct Config {
    pub name: String,
    pub remote: SocketAddr,
}
