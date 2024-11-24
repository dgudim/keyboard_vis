mod consts;
mod dbus;
mod utils;
use crate::consts::*;
use crate::dbus::*;
use crate::utils::*;
use atomic::Ordering;
use log::warn;
use log::{error, info};
use openrgb::data::Color;
use openrgb::data::Controller;
use openrgb::data::Mode;
use openrgb::data::LED;
use openrgb::OpenRGB;
use serde_json::Value;
use signal_hook::consts::SIGTERM;
use signal_hook::{consts::SIGINT, iterator::Signals};
use std::error::Error;
use std::fs;
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use std::vec;
use tokio::{net::TcpStream, time::sleep};

use rand::prelude::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    env_logger::init();

    // connect to default server at localhost
    let keyboard_client = get_openrgb_client("Keyboard client").await;
    let backlight_client = get_openrgb_client("Backlight client").await;

    let controller_ids = 0..keyboard_client.get_controller_count().await?;

    let mut keyboard_controller: Option<ControllerInfo> = None;
    let mut backlight_controller: Option<ControllerInfo> = None;

    // Read json config
    let config_j: Value = serde_json::from_str(
        fs::read_to_string("notification_config.json")
            .expect("Error reading notification config")
            .as_str(),
    )?;

    let keyboard_name = config_j["keyboard"]["name"]
        .as_str()
        .expect("Keyboard name missing");
    let keyboard_zone = config_j["keyboard"]["zone"]
        .as_str()
        .expect("Keyboard zone missing");
    let backlight_name = config_j["backlight"]["name"]
        .as_str()
        .expect("Backlight name missing");
    let backlight_zone = config_j["backlight"]["zone"]
        .as_str()
        .expect("Backlight zone missing");

    // query and print each controller data
    for controller_id in controller_ids {
        let controller = keyboard_client.get_controller(controller_id).await?;
        info!(
            "controller {}: {} | Zones: {:?}",
            controller_id,
            controller.name,
            controller
                .zones
                .iter()
                .map(|zone| { zone.name.as_str() })
                .collect::<Vec<_>>()
        );
        switch_mode(&keyboard_client, &controller, controller_id, "direct").await?;
        if controller.name.eq(keyboard_name) {
            turn_off_unused_zones(&keyboard_client, keyboard_zone, &controller, controller_id)
                .await?;
            keyboard_controller = Some(ControllerInfo::new(
                controller,
                controller_id,
                keyboard_zone,
            )?);
        } else if controller.name.eq(backlight_name) {
            turn_off_unused_zones(&keyboard_client, backlight_zone, &controller, controller_id)
                .await?;
            backlight_controller = Some(ControllerInfo::new(
                controller,
                controller_id,
                backlight_zone,
            )?);
        } else {
            turn_off_unused_zones(&keyboard_client, "", &controller, controller_id).await?;
        }
    }

    let keyboard_controller =
        Arc::new(keyboard_controller.unwrap_or_else(|| panic!("{} not found!", keyboard_name)));

    let backlight_controller =
        Arc::new(backlight_controller.unwrap_or_else(|| panic!("{} not found!", backlight_name)));

    // Starting frame: full black
    *KEYBOARD_BASE_FRAME.write().unwrap() = vec![BLACK; keyboard_controller.total_leds];
    *KEYBOARD_LAST_FRAME.write().unwrap() = vec![BLACK; keyboard_controller.total_leds];

    // Target frame: colored according to my preferences
    let keyboard_target_substrate = get_frame_by_key_names(
        keyboard_controller.leds(),
        Vec::from([
            KeyMap {
                keys: Vec::from(["Key: Number Pad", "Key: Num Lock"]),
                color: NUM_PAD_COLOR,
            },
            KeyMap {
                keys: Vec::from(["Insert", "Delete", "Page", "Arrow", "End", "Home"]),
                color: FUNCTION_COLOR,
            },
            KeyMap {
                keys: Vec::from(["Print", "Scroll", "Pause"]),
                color: FUNCTION_COLOR2,
            },
        ]),
        &|_: &LED, index: usize| match index <= 14 {
            true => TOP_ROW_COLOR,
            false => MAIN_COLOR,
        },
    );

    let mut signals = Signals::new([SIGINT, SIGTERM])?;
    thread::spawn({
        let keyboard_controller_arc = keyboard_controller.clone();

        move || {
            signals.forever().next(); // Blocks until the signal is received
            info!("Exiting main render loop...");
            let base = vec![BLACK; keyboard_controller_arc.total_leds];
            let mut rng = rand::thread_rng();
            for i in 1..7 {
                let frame = base
                    .iter()
                    .map(|_| {
                        let r: f64 = rng.gen();
                        Color {
                            b: 0,
                            g: 0,
                            r: (r / i as f64 * 255.0) as u8,
                        }
                    })
                    .collect();
                fade_into_frame(&frame, FRAME_DURATION_MS * 3);
            }
            fade_into_frame(&base, FRAME_DURATION_MS * 7);
            ABOUT_TO_SHUTDOWN.store(1, Ordering::Relaxed);
        }
    });

    tokio::spawn({
        let keyboard_controller_arc = keyboard_controller.clone();

        async move {
            info!("Started main render loop");
            match render_keyboard_frames(
                keyboard_controller_arc.id,
                keyboard_controller_arc.zone_id,
                &keyboard_client,
            )
            .await
            {
                Ok(_) => {
                    info!("Main loop exited, exiting the program");
                    ABOUT_TO_SHUTDOWN.store(2, Ordering::Relaxed);
                }
                Err(e) => {
                    error!("An error occurred in the frame rendering loop: {}", e);
                }
            };
        }
    });

    tokio::spawn(async move {
        info!("Started aux render loop");

        match render_backlight_frames(&backlight_controller, &backlight_client).await {
            Ok(_) => {}
            Err(e) => {
                error!("An error occurred in the aux frame rendering loop: {}", e);
            }
        };
    });

    let keyboard_gray_substrate = vec![GRAY; keyboard_controller.total_leds];

    for target_dist in 0..keyboard_controller.center_x * 3 {
        let target_dist_f = target_dist as f64;

        let intermediate: Frame = keyboard_gray_substrate
            .iter()
            .enumerate()
            .map(|(index, gray)| {
                let pos = keyboard_controller.num2xy(index);
                let distance_from_center = (((pos.x as i64 - keyboard_controller.center_x as i64)
                    .pow(2)
                    + (pos.y as i64 - keyboard_controller.center_y as i64).pow(2))
                    as f64)
                    .sqrt();

                // center color to gray
                if distance_from_center < target_dist_f {
                    let distance_factor = (distance_from_center - target_dist_f + 7.0) / 4.0; // 7 led offset from the center, 4 led width (offset from the edge)
                    return lerp_color(&keyboard_target_substrate[index], gray, distance_factor);
                }

                let distance_factor = (distance_from_center - target_dist_f) / 2.0;
                lerp_color(&WHITE, &BLACK, distance_factor)
            })
            .collect();

        fade_into_frame(&intermediate, FRAME_DURATION_MS * 2) // stretch each frame 2 times
    }

    *KEYBOARD_BASE_FRAME.write().unwrap() = keyboard_target_substrate;

    loop {
        match process_dbus(&config_j, keyboard_controller.clone()) {
            Ok(_) => return Ok(()),
            Err(_) => tokio::time::sleep(Duration::from_secs(1)).await,
        };
    }
}

async fn switch_mode(
    client: &OpenRGB<TcpStream>,
    controller: &Controller,
    controller_id: u32,
    mode_name: &str,
) -> Result<(), Box<dyn Error>> {
    let mode_name_l = mode_name.to_lowercase();

    info!("Switching {} to {mode_name_l} mode", controller.name);

    let (index, mode) = &controller
        .modes
        .iter()
        .enumerate()
        .find(|(_, mode)| mode.name.to_lowercase().eq(mode_name_l.as_str()))
        .expect("Didn't find {mode_name} for controller!");

    client
        .update_mode(
            controller_id,
            *index as i32,
            Mode {
                value: mode.value,
                brightness: mode.brightness,
                brightness_max: mode.brightness_max,
                brightness_min: mode.brightness_min,
                color_mode: mode.color_mode,
                colors: mode.colors.clone(),
                colors_max: mode.colors_max,
                colors_min: mode.colors_min,
                direction: mode.direction,
                flags: mode.flags,
                name: mode.name.clone(),
                speed: mode.speed,
                speed_max: mode.speed_max,
                speed_min: mode.speed_min,
            },
        )
        .await?;
    Ok(())
}

async fn turn_off_unused_zones(
    client: &OpenRGB<TcpStream>,
    whitelisted_zone: &str,
    controller: &Controller,
    controller_id: u32,
) -> Result<(), Box<dyn Error>> {
    if controller.zones.len() == 1 && whitelisted_zone.is_empty() {
        info!("Turning off controller: {}", controller.name);
        client
            .update_leds(controller_id, vec![BLACK; controller.leds.len()])
            .await?;
        return Ok(());
    }
    for (zone_id, z) in controller
        .zones
        .iter()
        .enumerate()
        .filter(|(_, z)| z.name.ne(whitelisted_zone))
    {
        info!(
            "Turning off zone {} of controller: {}",
            z.name, controller.name
        );
        client
            .update_zone_leds(
                controller_id,
                zone_id as u32,
                vec![BLACK; z.leds_count as usize],
            )
            .await?;
    }
    Ok(())
}

fn enq_keyboard_frame(frame: Frame) {
    *KEYBOARD_LAST_FRAME.write().unwrap() = frame.clone();
    match KEYBOARD_FRAME_Q.push(frame) {
        Ok(_) => {}
        Err(e) => {
            error!("Error adding frame! ({})", e);
        }
    }
}

async fn render_keyboard_frames(
    id: u32,
    zone_id: u32,
    client: &OpenRGB<TcpStream>,
) -> Result<(), Box<dyn Error>> {
    let frame_delay = Duration::from_millis(FRAME_DURATION_MS as u64);
    loop {
        match KEYBOARD_FRAME_Q.pop() {
            Ok(frame) => client.update_zone_leds(id, zone_id, frame).await?,
            Err(_) => {
                if ABOUT_TO_SHUTDOWN.load(Ordering::Relaxed) > 0 {
                    // Exit the loop, we need to shutdown
                    return Ok(());
                }
            }
        }

        sleep(frame_delay).await;
    }
}

async fn render_backlight_frames(
    backlight_controller: &ControllerInfo,
    client: &OpenRGB<TcpStream>,
) -> Result<(), Box<dyn Error>> {
    let frame_delay = Duration::from_millis(FRAME_DURATION_MS as u64);
    let base = vec![BLACK; backlight_controller.total_leds];

    let update_leds = |frame: Frame| {
        return client.update_zone_leds(
            backlight_controller.id,
            backlight_controller.zone_id,
            frame,
        );
    };

    let generate_frame = |offset: f64, offset2: f64, brightness: f64| {
        return base
            .iter()
            .enumerate()
            .map(|(index, _)| {
                lerp_color(
                    &BLACK,
                    &lerp_color(
                        &BACKLIGHT_WAVE1_COLOR,
                        &BACKLIGHT_WAVE2_COLOR,
                        ((index as f64 / 4.0 + offset).sin() * offset2.sin() + 1.0) / 2.0,
                    ),
                    brightness,
                )
            })
            .collect::<Vec<_>>();
    };

    let mut offset = 0.0;
    let mut offset2 = 0.8;
    let mut brightness = 0.0;

    loop {
        offset += 0.06;
        offset2 += 0.035;
        if SCREEN_LOCKED.load(Ordering::Relaxed) {
            brightness -= 0.07_f64
        } else if ABOUT_TO_SHUTDOWN.load(Ordering::Relaxed) > 0 {
            brightness -= 0.1_f64
        } else {
            brightness += 0.07_f64
        }
        brightness = brightness.clamp(0.0, 1.0);
        if brightness > 0.0 {
            update_leds(generate_frame(offset, offset2, brightness)).await?;
        }
        sleep(frame_delay).await;
    }
}

async fn get_openrgb_client(name: &str) -> OpenRGB<TcpStream> {
    loop {
        match OpenRGB::connect().await {
            Ok(cl) => {
                cl.set_name(name)
                    .await
                    .expect("Failed setting openrgb client name");
                info!("Connected to openrgb with name: {name}!");
                return cl;
            }
            Err(e) => {
                warn!("{}, retrying in 3 seconds", e);
                tokio::time::sleep(Duration::from_secs(3)).await
            }
        };
    }
}
