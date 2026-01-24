use crate::model::{MonitorStatus, ProbeRecord, Target, Protocol, AppConfig};
use crate::config;
use dashmap::DashMap;
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio::time::{sleep, Duration, Instant};
use chrono::Local;
use tokio::sync::{mpsc, watch};
use trust_dns_resolver::TokioAsyncResolver;
use std::net::IpAddr;

// Protocol Probes
#[async_trait::async_trait]
trait Probe {
    async fn probe(&self, target: &Target) -> (bool, Option<f32>, Option<String>);
}

struct TcpProbe;
#[async_trait::async_trait]
impl Probe for TcpProbe {
    async fn probe(&self, target: &Target) -> (bool, Option<f32>, Option<String>) {
        let port = target.port.unwrap_or(80);
        let addr = format!("{}:{}", target.host, port);
        let start = Instant::now();
        
        match tokio::time::timeout(Duration::from_secs(3), TcpStream::connect(&addr)).await {
            Ok(Ok(_)) => (true, Some(start.elapsed().as_micros() as f32 / 1000.0), None),
            Ok(Err(e)) => (false, None, Some(e.to_string())),
            Err(_) => (false, None, Some("Timeout".to_string())),
        }
    }
}

struct IcmpProbe;
#[async_trait::async_trait]
impl Probe for IcmpProbe {
    async fn probe(&self, target: &Target) -> (bool, Option<f32>, Option<String>) {
        // ICMP requires raw socket, might fail without root.
        // surge-ping 0.8 usage:
        // Pinger::new(_host)?.ping(seq, identifier, payload)
        
        // 解析 IP
        let ip = match target.host.parse::<IpAddr>() {
            Ok(ip) => ip,
            Err(_) => {
                // 尝试解析域名
                 match TokioAsyncResolver::tokio_from_system_conf() {
                     Ok(resolver) => {
                         match resolver.lookup_ip(target.host.as_str()).await {
                             Ok(ips) => if let Some(ip) = ips.iter().next() { ip } else { return (false, None, Some("DNS resolution failed".into())); },
                             Err(e) => return (false, None, Some(format!("DNS error: {}", e))),
                         }
                     }
                     Err(_) => return (false, None, Some("Resolver init failed".into())),
                 }
            }
        };

        let payload = [0; 8];
        match surge_ping::ping(ip, &payload).await {
            Ok((_, duration)) => (true, Some(duration.as_micros() as f32 / 1000.0), None),
            Err(e) => (false, None, Some(e.to_string())),
        }
    }
}

use trust_dns_resolver::config::{ResolverConfig, ResolverOpts, NameServerConfig, Protocol as DnsProtocol};
use std::net::SocketAddr;
use once_cell::sync::Lazy;

static HTTP_CLIENT: Lazy<reqwest::Client> = Lazy::new(|| {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .danger_accept_invalid_certs(true) // Allow self-signed certs for monitoring flexibility
        .no_gzip() // We only care about headers/status mostly
        .user_agent("NetWatch/0.3.0")
        .build()
        .expect("Failed to build HTTP client")
});

struct HttpProbe;
#[async_trait::async_trait]
impl Probe for HttpProbe {
    async fn probe(&self, target: &Target) -> (bool, Option<f32>, Option<String>) {
        let protocol = match target.protocol {
            Protocol::Https => "https",
            _ => "http",
        };
        
        let host = if target.host.contains("://") {
            target.host.clone()
        } else {
            let port_str = target.port.map(|p| format!(":{}", p)).unwrap_or_default();
            format!("{}://{}{}", protocol, target.host, port_str)
        };

        let start = Instant::now();
        match HTTP_CLIENT.get(&host).send().await {
            Ok(res) => {
                let duration = start.elapsed().as_micros() as f32 / 1000.0;
                let status = res.status();
                if status.is_success() {
                    (true, Some(duration), Some(format!("Status: {}", status)))
                } else {
                    (false, None, Some(format!("HTTP Error: {}", status)))
                }
            },
            Err(e) => (false, None, Some(e.to_string())),
        }
    }
}

struct DnsProbe;
#[async_trait::async_trait]
impl Probe for DnsProbe {
    async fn probe(&self, target: &Target) -> (bool, Option<f32>, Option<String>) {
        // Parse target host as IP for custom Name Server
        let ip = match target.host.parse::<IpAddr>() {
            Ok(ip) => ip,
            Err(e) => return (false, None, Some(format!("Invalid DNS Server IP: {}", e))),
        };
        
        let port = target.port.unwrap_or(53);
        let socket_addr = SocketAddr::new(ip, port);
        
        // Configure resolver to use the target as Name Server
        let mut config = ResolverConfig::new();
        config.add_name_server(NameServerConfig::new(
            socket_addr,
            DnsProtocol::Udp,
        ));
        
        // Create resolver
        let resolver = match TokioAsyncResolver::tokio(config, ResolverOpts::default()) {
             r => r,
             // Err(e) => return (false, None, Some(format!("Resolver init failed: {}", e))),
        };

        // Query a stable domain (e.g., www.baidu.com)
        let query_domain = "www.baidu.com";
        let start = Instant::now();
        
        match tokio::time::timeout(Duration::from_secs(3), resolver.lookup_ip(query_domain)).await {
            Ok(Ok(ips)) => {
                let duration = start.elapsed().as_micros() as f32 / 1000.0;
                let result = ips.iter().map(|ip| ip.to_string()).collect::<Vec<_>>().join(", ");
                (true, Some(duration), Some(result))
            },
            Ok(Err(e)) => (false, None, Some(e.to_string())),
            Err(_) => (false, None, Some("Timeout".to_string())),
        }
    }
}

// ----------------------------------------------------------------

pub enum MonitorEvent {
    StateChanged(String, bool), // id, new_state
}

pub async fn start_monitor_loop(
    state: Arc<DashMap<String, MonitorStatus>>,
    tx: mpsc::Sender<MonitorEvent>,
    mut config_rx: watch::Receiver<AppConfig>,
) {
    tracing::info!("Starting monitoring engine...");
    
    // 我们需要一个循环来管理"探测循环"。
    // 当配置更新时，我们需要更新探测列表。
    // 简单的做法是：每次循环检查是否有配置更新，如果有，更新本地 targets 列表。
    
    // 初始化 targets
    let mut current_targets_hash = hash_targets(&config_rx.borrow().targets);
    let mut targets = config_rx.borrow().targets.clone();

    // 初始同步
    {
        let new_ids: std::collections::HashSet<String> = targets.iter().map(|t| t.id.clone()).collect();
        state.retain(|k, _| new_ids.contains(k));
        for target in &targets {
            if !state.contains_key(&target.id) {
                tracing::debug!("Initializing monitor for {}: last_known_state={:?}", target.name, target.last_known_state);
                state.insert(target.id.clone(), MonitorStatus::new(target.clone()));
            } else if let Some(mut entry) = state.get_mut(&target.id) {
                 entry.value_mut().target = target.clone();
            }
        }
    }

    loop {
        // 1. 执行探测
        let mut handles = vec![];
        let retention_days = config_rx.borrow().data_retention_days; // 获取 retention
        
        for target in targets.clone() {
            let state_clone = state.clone();
            let tx_clone = tx.clone();
            let alert_config = config_rx.borrow().alert.clone();

            handles.push(tokio::spawn(async move {
                probe_target(&state_clone, target, tx_clone, &alert_config, retention_days).await;
            }));
        }
        
        // 等待所有探测完成
        for handle in handles {
            let _ = handle.await;
        }

        // 2. 休眠或等待配置变更
        let reload = tokio::select! {
            _ = sleep(Duration::from_secs(10)) => {
                false
            },
            res = config_rx.changed() => {
                res.is_ok()
            }
        };

        if reload {
            let new_config = config_rx.borrow_and_update();
            let new_targets_hash = hash_targets(&new_config.targets);
            
            // 只有当 targets 列表的关键属性发生变化时才重载
            if new_targets_hash != current_targets_hash {
                tracing::info!("Configuration changed, reloading monitors...");
                current_targets_hash = new_targets_hash;
                targets = new_config.targets.clone();
                
                // Sync DashMap
                let new_ids: std::collections::HashSet<String> = targets.iter().map(|t| t.id.clone()).collect();
                state.retain(|k, _| new_ids.contains(k));
                for target in &targets {
                    if !state.contains_key(&target.id) {
                        state.insert(target.id.clone(), MonitorStatus::new(target.clone()));
                    } else if let Some(mut entry) = state.get_mut(&target.id) {
                         entry.value_mut().target = target.clone();
                    }
                }
            } else {
                // 如果只是状态变更（例如持久化任务写回了 last_known_state），
                // 或者是 retention days 变更，我们不需要完全重载 targets，
                // 但我们可能需要更新 loop 中的 retention days 变量（下一轮循环会自动获取）。
                // 所以这里什么都不做，或者只更新 targets 变量以防万一。
                targets = new_config.targets.clone();
            }
        }
    }
}

// 简单的 hash 函数，用于比较 targets 是否实质性变更
fn hash_targets(targets: &[Target]) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    
    let mut hasher = DefaultHasher::new();
    // 只 hash 影响探测的关键字段：id, host, port, protocol
    for t in targets {
        t.id.hash(&mut hasher);
        t.host.hash(&mut hasher);
        t.port.hash(&mut hasher);
        t.protocol.hash(&mut hasher);
        // 注意：不 hash last_known_state
    }
    hasher.finish()
}

async fn probe_target(
    state: &Arc<DashMap<String, MonitorStatus>>, 
    target: Target,
    tx: mpsc::Sender<MonitorEvent>,
    alert_config: &crate::model::AlertConfig,
    retention_days: u64,
) {
    // 根据协议选择 Probe
    let probe_impl: Box<dyn Probe + Send + Sync> = match target.protocol {
        Protocol::Tcp => Box::new(TcpProbe),
        Protocol::Icmp => Box::new(IcmpProbe),
        Protocol::Dns => Box::new(DnsProbe),
        Protocol::Http | Protocol::Https => Box::new(HttpProbe),
    };

    let (success, latency, message) = probe_impl.probe(&target).await;

    if let Some(mut entry) = state.get_mut(&target.id) {
        let status = entry.value_mut();
        
        // 如果是第一次探测（records 为空），且当前探测成功，且当前状态为 false（默认），
        // 且 target.last_known_state 为 None 或 false，
        // 这通常意味着这是程序启动后的第一次成功探测。
        // 为了避免产生 "false -> true" 的日志（如果我们认为它本来就该是 true），
        // 我们可以静默更新状态，不触发 Event。
        // 但如果 last_known_state 是 true，MonitorStatus::new 会初始化为 true，所以不会触发变更。
        // 所以问题在于：新添加的 target 默认 last_known_state 是 None -> false。
        // 当第一次探测成功时，会 false -> true。
        // 用户希望这被视为"初始状态确认"，而不是"变更"。
        // 我们可以增加一个字段 initialized? 或者判断 records.len() == 0。
        // 这里我们在 push record 之前判断。
        
        let is_first_record = status.records.is_empty();

        let record = ProbeRecord {
            timestamp: Local::now(),
            latency_ms: latency,
            success,
            message: message.clone(),
        };
        
        status.records.push_front(record);
        
        // 计算 limit: days * 24h * 60m * 6 (10s interval)
        // 10s interval is hardcoded in loop currently.
        let limit = retention_days * 24 * 3600 / 10;
        let limit = if limit == 0 { 60 } else { limit as usize }; // 至少保留一点

        if status.records.len() > limit {
            status.records.pop_back();
        }

        // 防抖动逻辑：连续失败/成功 3 次才切换状态
        // 但是这里我们只存储了 ProbeRecord，每次 probe 只产生一个 record。
        // 我们需要检查最近 3 次记录。
        // 如果当前状态是 UP (true)，我们需要连续 3 次失败 (false) 才切换为 DOWN。
        // 如果当前状态是 DOWN (false)，我们需要连续 3 次成功 (true) 才切换为 UP。
        
        let check_count = 3;
        let mut should_switch = false;
        
        if is_first_record {
            // 首次探测特殊处理：
            // 1. 如果 last_known_state 为 None，我们需要确立初始状态并持久化（无论成功失败）。
            // 2. 如果探测结果与当前默认状态不一致，需要修正并持久化。
            if target.last_known_state.is_none() || status.current_state != success {
                tracing::info!("Initial state confirmed for {}: {}", target.name, if success { "UP" } else { "DOWN" });
                status.current_state = success;
                let _ = tx.send(MonitorEvent::StateChanged(target.id.clone(), success)).await;
            }
        } else if status.records.len() >= check_count {
             let recent_records: Vec<bool> = status.records.iter().take(check_count).map(|r| r.success).collect();
             if recent_records.len() == check_count {
                 // 如果所有最近记录都与当前状态相反，则切换
                 // 比如当前是 true，最近是 [false, false, false]，则切换。
                 if recent_records.iter().all(|&s| s != status.current_state) {
                     should_switch = true;
                 }
             }
        } else {
              // 初始阶段，数据不足 3 次，如果状态不一致，直接切换（快速启动）
              if status.current_state != success {
                  should_switch = true;
              }
         }

        if should_switch {
            tracing::info!("State changed for {}: {} -> {}", target.name, status.current_state, !status.current_state);
            status.current_state = !status.current_state;
            
            // 1. 发送 Webhook
            let target_clone = target.clone();
            let alert_config_clone = alert_config.clone();
            let message_clone = message.clone();
            
            if alert_config_clone.enabled {
                 tokio::spawn(async move {
                    let _ = crate::alert::send_alert(&target_clone, success, &alert_config_clone, message_clone.as_deref()).await;
                });
            }

            // 2. 触发持久化
            let _ = tx.send(MonitorEvent::StateChanged(target.id.clone(), success)).await;
        }
    }
}

pub async fn config_persistence_task(
    mut rx: mpsc::Receiver<MonitorEvent>,
    state: Arc<DashMap<String, MonitorStatus>>,
    config_watch: watch::Sender<AppConfig>, // 用于获取最新配置
) {
    while let Some(MonitorEvent::StateChanged(id, new_state)) = rx.recv().await {
        // 更新内存状态
        if let Some(mut entry) = state.get_mut(&id) {
            entry.value_mut().target.last_known_state = Some(new_state);
        }

        // 获取当前完整配置快照
        let mut current_config = config_watch.borrow().clone();
        
        // 更新 targets 中的状态
        for t in &mut current_config.targets {
            if t.id == id {
                t.last_known_state = Some(new_state);
                break;
            }
        }
        
        // 保存
        if let Err(e) = config::save_config(&current_config) {
            tracing::error!("Failed to save config with new state: {}", e);
        } else {
             // 关键修复：保存后必须更新 config_watch 中的值，否则下次获取的还是旧的 initial_config，
             // 导致其他 target 的状态回退为 null。
             // 使用 send_if_modified 或 send 都可以，但要注意这会触发 loop 的 changed()。
             // 我们已经在 loop 中处理了 hash 比较，所以这里触发是安全的。
             let _ = config_watch.send(current_config);
             
            tracing::info!("Config saved with updated state for target {}", id);
        }
    }
}
