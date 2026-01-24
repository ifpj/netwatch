use crate::model::{AppConfig, Target, Protocol};
use std::fs;
use std::path::Path;
use anyhow::Context;
use once_cell::sync::OnceCell;

static CONFIG_PATH: OnceCell<String> = OnceCell::new();

pub fn init_config_path(path: String) {
    if CONFIG_PATH.set(path).is_err() {
        tracing::warn!("Config path already initialized");
    }
}

fn get_config_path() -> &'static str {
    CONFIG_PATH.get().map(|s| s.as_str()).unwrap_or("config.json")
}

pub fn load_config() -> anyhow::Result<AppConfig> {
    let path = get_config_path();
    if !Path::new(path).exists() {
        tracing::info!("Config file not found at {}, creating default.", path);
        let defaults = get_default_config();
        save_config(&defaults)?;
        return Ok(defaults);
    }

    let content = fs::read_to_string(path).context("Failed to read config file")?;
    match serde_json::from_str::<AppConfig>(&content) {
        Ok(config) => Ok(config),
        Err(e) => {
             // Fallback or error handling
             anyhow::bail!("Failed to parse config file: {}", e);
        }
    }
}

pub fn save_config(config: &AppConfig) -> anyhow::Result<()> {
    let path = get_config_path();
    let content = serde_json::to_string_pretty(config)?;
    let tmp_file = format!("{}.tmp", path);
    fs::write(&tmp_file, content).context("Failed to write temp config file")?;
    fs::rename(&tmp_file, path).context("Failed to replace config file")?;
    Ok(())
}

fn get_default_config() -> AppConfig {
    AppConfig {
        targets: vec![
            Target {
                id: "1".to_string(),
                host: "8.8.8.8".to_string(),
                port: Some(53),
                name: "Google DNS (TCP)".to_string(),
                protocol: Protocol::Tcp,
                last_known_state: None,
            },
            Target {
                id: "2".to_string(),
                host: "1.1.1.1".to_string(),
                port: None,
                name: "Cloudflare Ping".to_string(),
                protocol: Protocol::Icmp,
                last_known_state: None,
            },
             Target {
                id: "3".to_string(),
                host: "8.8.8.8".to_string(),
                port: Some(53),
                name: "Google DNS Query".to_string(),
                protocol: Protocol::Dns,
                last_known_state: None,
            },
            Target {
                id: "4".to_string(),
                host: "www.google.com".to_string(),
                port: None,
                name: "Google Web (HTTPS)".to_string(),
                protocol: Protocol::Https,
                last_known_state: None,
            },
        ],
        alert: Default::default(),
        data_retention_days: 3,
    }
}
