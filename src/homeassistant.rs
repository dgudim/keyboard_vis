use std::time::Duration;

use atomic::Ordering;
use log::{info, warn};
use serde_json::Value;

use crate::consts::*;
use crate::enq_keyboard_frame;

pub struct HomeAssistantConfig {
    pub url: String,
    pub token: String,

    pub light_sensor_id: String,
    pub lux_threshold: f64,
    pub poll_interval_seconds: u64,
    pub dim_brightness_mult: f64,
}

impl HomeAssistantConfig {
    pub fn from_config(config_j: &Value) -> Option<HomeAssistantConfig> {
        let ha = config_j.get("home_assistant")?;
        if ha.is_null() {
            return None;
        }

        let url = ha["url"].as_str()?.trim_end_matches('/').to_string();
        let light_sensor_id = ha["sensor_id"].as_str()?.to_string();

        Some(HomeAssistantConfig {
            url,
            light_sensor_id,
            token: ha["token"].as_str().unwrap_or("").to_string(),
            lux_threshold: ha["lux_threshold"].as_f64().unwrap_or(70.0),
            poll_interval_seconds: ha["poll_interval_seconds"].as_u64().unwrap_or(30),
            dim_brightness_mult: ha["dim_brightness_mult"].as_f64().unwrap_or(0.3).clamp(0.0, 1.0),
        })
    }
}

pub fn spawn_ambient_light_monitor(config: HomeAssistantConfig) {
    tokio::spawn(async move {
        let client = reqwest::Client::new();
        let endpoint = format!("{}/api/states/{}", config.url, config.light_sensor_id);

        info!(
            "Home Assistant ambient light monitor started: {endpoint} (dim below {} lux to {:.2}x, polling every {}s)",
            config.lux_threshold, config.dim_brightness_mult, config.poll_interval_seconds
        );

        if config.token.is_empty() {
            warn!("No Home Assistant token configured; the REST API will likely reject requests with 401");
        }

        loop {
            match fetch_lux(&client, &endpoint, &config.token).await {
                Ok(lux) => {
                    let target = if lux < config.lux_threshold {
                        config.dim_brightness_mult
                    } else {
                        1.0
                    };
                    let current = AMBIENT_BRIGHTNESS.load(Ordering::Relaxed);
                    if (current - target).abs() > f64::EPSILON {
                        info!("Ambient light {lux:.1} lux -> target brightness {target:.2}");
                        fade_ambient_brightness(target).await;
                    }
                }
                Err(e) => warn!("Could not read light sensor from Home Assistant: {e}"),
            }

            tokio::time::sleep(Duration::from_secs(config.poll_interval_seconds)).await;
        }
    });
}


async fn fade_ambient_brightness(target: f64) {
    const STEPS: u32 = 12;
    let start = AMBIENT_BRIGHTNESS.load(Ordering::Relaxed);

    for step in 1..=STEPS {
        let value = start + (target - start) * (step as f64 / STEPS as f64);
        AMBIENT_BRIGHTNESS.store(value, Ordering::Relaxed);

        let last = KEYBOARD_LAST_FRAME.read().unwrap().clone();
        if !last.is_empty() {
            enq_keyboard_frame(last);
        }

        tokio::time::sleep(Duration::from_millis(FRAME_DURATION_MS as u64)).await;
    }

    AMBIENT_BRIGHTNESS.store(target, Ordering::Relaxed);
}

async fn fetch_lux(
    client: &reqwest::Client,
    endpoint: &str,
    token: &str,
) -> Result<f64, Box<dyn std::error::Error + Send + Sync>> {
    let mut request = client.get(endpoint);
    if !token.is_empty() {
        request = request.bearer_auth(token);
    }

    let body: Value = request.send().await?.error_for_status()?.json().await?;

    let state = body["state"]
        .as_str()
        .ok_or("`state` field missing or not a string")?;

    let lux: f64 = state.trim().parse()?;
    Ok(lux)
}
