use serde::{Deserialize, Serialize};
use chrono::{DateTime, Local};
use std::collections::VecDeque;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "UPPERCASE")]
pub enum Protocol {
    Tcp,
    Icmp,
    Dns,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Target {
    pub id: String,
    pub host: String, // IP or Domain
    pub port: Option<u16>, // ICMP 不需要端口
    pub name: String,
    #[serde(default = "default_proto")]
    pub protocol: Protocol,
    
    // 状态持久化
    #[serde(default)]
    pub last_known_state: Option<bool>,
}

fn default_proto() -> Protocol { Protocol::Tcp }

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AlertConfig {
    pub enabled: bool,
    #[serde(default)]
    pub webhooks: Vec<WebhookConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookConfig {
    #[serde(default = "generate_uuid")]
    pub id: String,
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub template: Option<String>, // Optional override
    pub enabled: bool,
}

fn generate_uuid() -> String {
    Uuid::new_v4().to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub targets: Vec<Target>,
    pub alert: AlertConfig,
    #[serde(default = "default_retention_days")]
    pub data_retention_days: u64,
}

fn default_retention_days() -> u64 { 3 }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeRecord {
    pub timestamp: DateTime<Local>,
    pub latency_ms: Option<f32>,
    pub success: bool,
    pub message: Option<String>, // 错误信息或 DNS 解析结果
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitorStatus {
    pub target: Target,
    pub records: VecDeque<ProbeRecord>,
    pub current_state: bool,
}

impl MonitorStatus {
    pub fn new(target: Target) -> Self {
        let initial_state = target.last_known_state.unwrap_or(false);
        Self {
            target,
            records: VecDeque::with_capacity(60),
            current_state: initial_state,
        }
    }
}
