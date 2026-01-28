use crate::model::{AlertConfig, Target};
use serde_json::json;

pub async fn send_alert(
    target: &Target,
    is_online: bool,
    config: &AlertConfig,
    extra_msg: Option<&str>,
) -> anyhow::Result<()> {
    if !config.enabled {
        return Ok(());
    }

    let status_text = if is_online { "🟢 UP" } else { "🔴 DOWN" };
    let _status_icon = if is_online { "🟢" } else { "🔴" };
    let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let detail = extra_msg.unwrap_or("");

    // Generic Webhooks
    let client = reqwest::Client::new();

    for webhook in &config.webhooks {
        if !webhook.enabled {
            continue;
        }

        let url = &webhook.url;
        if url.is_empty() {
            continue;
        }

        // 如果有模板，使用模板替换
        let payload = if let Some(tmpl) = &webhook.template {
            let mut body = tmpl.clone();
            body = body.replace("{{TARGET}}", &target.name);
            body = body.replace("{{HOST}}", &target.host);
            body = body.replace("{{STATUS}}", status_text);
            body = body.replace("{{TIME}}", &timestamp);
            body = body.replace("{{MESSAGE}}", detail);

            match serde_json::from_str::<serde_json::Value>(&body) {
                Ok(v) => v,
                Err(_) => json!({ "text": body }), // Fallback
            }
        } else {
            // 默认 JSON Payload
            json!({
                "target": target.name,
                "host": target.host,
                "status": status_text,
                "timestamp": timestamp,
                "message": detail
            })
        };

        let client = client.clone();
        let url = url.clone();

        tokio::spawn(async move {
            tracing::debug!("Sending webhook to {}", url);
            match client.post(&url).json(&payload).send().await {
                Ok(res) => {
                    if !res.status().is_success() {
                        tracing::error!("Webhook failed with status {}: {}", res.status(), url);
                        if let Ok(text) = res.text().await {
                            tracing::error!("Response body: {}", text);
                        }
                    } else {
                        tracing::debug!("Webhook sent successfully to {}", url);
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to send Webhook to {}: {}", url, e);
                }
            }
        });
    }

    Ok(())
}
