mod config;
mod model;
mod monitor;
mod web;
mod alert;

use dashmap::DashMap;
use std::sync::Arc;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use tokio::sync::{mpsc, watch, broadcast};
use web::AppState;
use std::env;

#[tokio::main]
async fn main() {
    // Parse command line arguments
    let args: Vec<String> = env::args().collect();
    let mut config_path = "config.json".to_string();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-d" => {
                if i + 1 < args.len() {
                    let dir = &args[i + 1];
                    if let Err(e) = env::set_current_dir(dir) {
                        eprintln!("Failed to change directory to {}: {}", dir, e);
                        std::process::exit(1);
                    }
                    i += 1;
                } else {
                    eprintln!("Missing argument for -d");
                    std::process::exit(1);
                }
            }
            "-c" => {
                if i + 1 < args.len() {
                    config_path = args[i + 1].clone();
                    i += 1;
                } else {
                    eprintln!("Missing argument for -c");
                    std::process::exit(1);
                }
            }
            _ => {}
        }
        i += 1;
    }

    // Initialize config path
    config::init_config_path(config_path);

    // Initialize logging
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "netwatch=info,tower_http=debug".into()),
        ))
        .with(tracing_subscriber::fmt::layer())
        .init();

    // 1. 加载配置
    let initial_config = match config::load_config() {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("Failed to load configuration: {}", e);
            return;
        }
    };

    // 2. 初始化 State
    let status_map = Arc::new(DashMap::new());
    for target in &initial_config.targets {
        status_map.insert(target.id.clone(), model::MonitorStatus::new(target.clone()));
    }

    // 尝试加载缓存
    load_cache(&status_map);

    // 3. 创建通道
    let (monitor_tx, monitor_rx) = mpsc::channel(100);
    let (config_tx, config_rx) = watch::channel(initial_config);
    let (broadcast_tx, _) = broadcast::channel(100);
    let (shutdown_tx, _) = broadcast::channel(1);

    // 4. 启动配置持久化任务 (Config Writer)
    let persistence_map = status_map.clone();
    let persistence_config_tx = config_tx.clone();
    let _persistence_config_rx = config_tx.subscribe(); // 获取一个新的 receiver
    
    // 修正: persistence task 需要的是 watch::Sender 来更新配置吗？
    // 不，persistence task 接收 StateChanged 事件，然后更新文件。
    // 为了更新文件，它需要完整的 AppConfig。
    // 它可以通过 config_rx.borrow() 获取。
    // 但是，我们还需要更新内存中的 Config (target.last_known_state)。
    // watch channel 是单向的。如果我们通过 persistence task 更新了 last_known_state 并 save_config，
    // 我们是否需要通知 monitor loop？Monitor loop 主要关心 targets 列表变更。
    // last_known_state 变更不需要触发 monitor loop 重载。
    // 所以 persistence task 只需要 save_config 即可。
    
    // 这里代码稍微调整一下，传入 config_tx 用于... 其实不需要 config_tx，只需要 save_config。
    // 但是 monitor.rs 的签名要改一下，不要传 watch::Sender，传 watch::Receiver 即可。
    // 等等，monitor.rs 的 config_persistence_task 签名我写的是 watch::Sender。
    // 让我们修正它。
    
    tokio::spawn(async move {
        monitor::config_persistence_task(monitor_rx, persistence_map, persistence_config_tx).await;
    });

    // 5. 启动后台探测任务 (Monitor Loop)
    let monitor_map = status_map.clone();
    let monitor_config_rx = config_rx.clone();
    let monitor_broadcast_tx = broadcast_tx.clone();
    tokio::spawn(async move {
        monitor::start_monitor_loop(monitor_map, monitor_tx, monitor_config_rx, monitor_broadcast_tx).await;
    });

    // 6. 启动 Web 服务
    let app_state = AppState {
        status_map: status_map.clone(),
        config_tx,
        config_rx,
        broadcast_tx,
        shutdown_tx: shutdown_tx.clone(),
    };
    
    let app = web::app(app_state);
    let addr = "0.0.0.0:3000";
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    tracing::info!("Web Server listening on http://{}", addr);
    
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal(status_map.clone(), shutdown_tx))
        .await
        .unwrap();
}

const CACHE_FILE: &str = "cache.json";

fn save_cache(state: &DashMap<String, model::MonitorStatus>) {
    tracing::info!("Saving monitor cache to {}", CACHE_FILE);
    let items: Vec<model::MonitorStatus> = state.iter().map(|v| v.value().clone()).collect();
    match serde_json::to_string(&items) {
        Ok(json) => {
            if let Err(e) = std::fs::write(CACHE_FILE, json) {
                 tracing::error!("Failed to write cache file: {}", e);
            }
        }
        Err(e) => tracing::error!("Failed to serialize cache: {}", e),
    }
}

fn load_cache(state: &DashMap<String, model::MonitorStatus>) {
    if !std::path::Path::new(CACHE_FILE).exists() {
        return;
    }
    tracing::info!("Loading monitor cache from {}", CACHE_FILE);
    match std::fs::read_to_string(CACHE_FILE) {
        Ok(content) => {
             match serde_json::from_str::<Vec<model::MonitorStatus>>(&content) {
                Ok(items) => {
                    for item in items {
                         // 我们只恢复 targets 列表中存在的 target 的状态
                         if let Some(mut existing) = state.get_mut(&item.target.id) {
                             existing.records = item.records;
                             existing.current_state = item.current_state;
                             tracing::info!("Restored cache for target: {}", item.target.name);
                         }
                    }
                }
                Err(e) => tracing::error!("Failed to parse cache file: {}", e),
             }
        }
        Err(e) => tracing::error!("Failed to read cache file: {}", e),
    }
}

async fn shutdown_signal(state: Arc<DashMap<String, model::MonitorStatus>>, shutdown_tx: broadcast::Sender<()>) {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    tracing::info!("Shutdown signal received, saving cache...");
    // Send shutdown signal to all SSE connections
    let _ = shutdown_tx.send(());
    save_cache(&state);
    tracing::info!("Goodbye!");
}
