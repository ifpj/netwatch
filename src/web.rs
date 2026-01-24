use axum::{
    routing::get,
    Router, Json, extract::State,
    response::IntoResponse,
    http::{header, StatusCode, Uri},
};
use std::sync::Arc;
use dashmap::DashMap;
use crate::model::{MonitorStatus, AppConfig};
use tokio::sync::watch;
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "static"]
struct Assets;

#[derive(Clone)]
pub struct AppState {
    pub status_map: Arc<DashMap<String, MonitorStatus>>,
    pub config_tx: watch::Sender<AppConfig>, // 用于更新配置
    pub config_rx: watch::Receiver<AppConfig>, // 用于获取当前配置
}

pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/api/status", get(get_status))
        .route("/api/config", get(get_config).post(update_config))
        .route("/", get(index_handler))
        .route("/index.html", get(index_handler))
        .route("/*file", get(static_handler))
        .with_state(state)
}

async fn index_handler() -> impl IntoResponse {
    static_handler(Uri::from_static("/index.html")).await
}

async fn static_handler(uri: Uri) -> impl IntoResponse {
    let mut path = uri.path().trim_start_matches('/').to_string();
    if path.starts_with("static/") {
        path = path.replace("static/", "");
    }
    if path.is_empty() {
        path = "index.html".to_string();
    }
    
    match Assets::get(path.as_str()) {
        Some(content) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            ([(header::CONTENT_TYPE, mime.as_ref())], content.data).into_response()
        }
        None => (StatusCode::NOT_FOUND, "404 Not Found").into_response(),
    }
}

async fn get_status(State(state): State<AppState>) -> Json<Vec<MonitorStatus>> {
    let mut status_list: Vec<MonitorStatus> = state.status_map.iter().map(|v| v.value().clone()).collect();
    // 按照配置中的顺序排序，而不是 ID 字母序，这样 UI 不会乱跳
    let config = state.config_rx.borrow();
    let order_map: std::collections::HashMap<String, usize> = config.targets.iter().enumerate().map(|(i, t)| (t.id.clone(), i)).collect();
    
    status_list.sort_by(|a, b| {
        let idx_a = order_map.get(&a.target.id).unwrap_or(&usize::MAX);
        let idx_b = order_map.get(&b.target.id).unwrap_or(&usize::MAX);
        idx_a.cmp(idx_b)
    });
    
    Json(status_list)
}

async fn get_config(State(state): State<AppState>) -> Json<AppConfig> {
    Json(state.config_rx.borrow().clone())
}

async fn update_config(
    State(state): State<AppState>,
    Json(mut new_config): Json<AppConfig>,
) -> Json<serde_json::Value> {
    // 0. Preserve last_known_state from memory
    // Because frontend might send null or outdated states (since it only fetches config once).
    // We should trust our in-memory status map (which has the latest probe results).
    for target in &mut new_config.targets {
        // First priority: Real-time status from Monitor Engine
        if let Some(entry) = state.status_map.get(&target.id) {
            target.last_known_state = Some(entry.value().current_state);
        } 
        // Second priority: If not in map (e.g. not initialized yet), check previous config
        else if target.last_known_state.is_none() {
             let current_config = state.config_rx.borrow();
             if let Some(old_target) = current_config.targets.iter().find(|t| t.id == target.id) {
                 target.last_known_state = old_target.last_known_state;
             }
        }
    }

    // 1. 保存到文件
    if let Err(e) = crate::config::save_config(&new_config) {
        return Json(serde_json::json!({ "success": false, "error": e.to_string() }));
    }

    // 2. 广播更新 (这会触发 Monitor Loop 重载)
    let _ = state.config_tx.send(new_config);

    Json(serde_json::json!({ "success": true }))
}
