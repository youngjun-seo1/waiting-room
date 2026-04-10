use serde::Deserialize;
use std::net::SocketAddr;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub listen_addr: SocketAddr,
    pub origin_url: String,
    pub max_active_users: u32,
    pub session_ttl_secs: u64,
    pub queue_cookie_name: String,
    pub admin_api_key: String,
    pub enabled: bool,
    #[serde(default)]
    pub redis_url: String,
    #[serde(default)]
    pub branding: BrandingConfig,
    #[serde(default)]
    pub advanced: AdvancedConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BrandingConfig {
    #[serde(default = "default_page_title")]
    pub page_title: String,
    #[serde(default)]
    pub logo_url: String,
}

impl Default for BrandingConfig {
    fn default() -> Self {
        Self {
            page_title: default_page_title(),
            logo_url: String::new(),
        }
    }
}

fn default_page_title() -> String {
    "Please wait...".to_string()
}

#[derive(Debug, Clone, Deserialize)]
pub struct AdvancedConfig {
    #[serde(default = "default_reaper_interval")]
    pub reaper_interval_secs: u64,
}

impl Default for AdvancedConfig {
    fn default() -> Self {
        Self {
            reaper_interval_secs: default_reaper_interval(),
        }
    }
}

fn default_reaper_interval() -> u64 {
    5
}

impl Config {
    pub fn load(path: &str) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let mut config: Config = toml::from_str(&content)?;

        // Environment variable overrides
        if let Ok(v) = std::env::var("WR_LISTEN_ADDR") {
            config.listen_addr = v.parse()?;
        }
        if let Ok(v) = std::env::var("WR_ORIGIN_URL") {
            config.origin_url = v;
        }
        if let Ok(v) = std::env::var("WR_MAX_ACTIVE_USERS") {
            config.max_active_users = v.parse()?;
        }
        if let Ok(v) = std::env::var("WR_SESSION_TTL_SECS") {
            config.session_ttl_secs = v.parse()?;
        }
        if let Ok(v) = std::env::var("WR_ADMIN_API_KEY") {
            config.admin_api_key = v;
        }
        if let Ok(v) = std::env::var("WR_ENABLED") {
            config.enabled = v.parse()?;
        }
        if let Ok(v) = std::env::var("WR_REDIS_URL") {
            config.redis_url = v;
        }

        Ok(config)
    }
}
