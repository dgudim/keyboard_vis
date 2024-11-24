use std::{
    error::Error,
    ops::AddAssign,
    sync::{atomic::Ordering, Arc, RwLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use css_color_parser::Color as CssColor;
use dashmap::DashMap;
use log::info;
use openrgb::data::{Color, Controller, ZoneType, LED};

use crate::{consts::*, enq_keyboard_frame};

pub struct ControllerInfo {
    pub raw: Controller,
    pub id: u32,
    pub zone_id: u32,

    pub width: usize,
    pub height: usize,

    pub center_x: usize,
    pub center_y: usize,

    pub total_leds: usize,
}

impl ControllerInfo {
    pub fn new(
        controller: Controller,
        id: u32,
        zone_name: &str,
    ) -> Result<ControllerInfo, Box<dyn Error>> {
        let (target_zone_id, target_zone) = controller
            .zones
            .iter()
            .enumerate()
            .find(|(_, zone)| zone.name.eq(zone_name))
            .expect("Zone {zone_name} not found in {id}");

        let mut height = 1;
        let total_leds = target_zone.leds_count as usize;
        let mut width = total_leds;

        if target_zone.r#type.eq(&ZoneType::Matrix) {
            let zone_matrix = target_zone
                .matrix
                .as_ref()
                .expect("Matrix missing for {zone_name}");
            width = zone_matrix.num_columns();
            height = zone_matrix.num_rows();
            if zone_matrix.num_elements() != total_leds {
                Err("zone_matrix.num_elements() != total_leds")?
            }
        }

        info!(
            "Constructed a new controller: {} 
                | zone name: {zone_name} 
                | total_leds: {total_leds} 
                | width: {width}, height: {height}
                | center x: {}, center y: {}",
            controller.name,
            width / 2,
            height / 2
        );

        Ok(ControllerInfo {
            zone_id: target_zone_id as u32,
            raw: controller,
            id,
            width,
            height,
            center_x: width / 2,
            center_y: height / 2,
            total_leds,
        })
    }

    pub fn leds(&self) -> impl Iterator<Item = (usize, &LED)> {
        return self.raw.leds.iter().enumerate();
    }

    // Index of the led into xy coordinates
    pub fn num2xy(&self, index: usize) -> Point {
        let nc = index.clamp(0, self.total_leds);
        let y = nc / self.width;
        let x = nc - y * self.width;
        Point {
            x,
            y: self.height - y - 1,
        }
    }
}

pub struct NotificationSettings {
    pub color: Color,
    pub important: bool,
    pub flash_on_notify: bool,
    pub flash_on_auto_close: Color,
}

pub struct Notification {
    pub id: u32,
    pub sender: String,
    pub settings: Arc<NotificationSettings>,
    pub timestamp: u128,
}

pub type ProgressMap = DashMap<String, (Color, f64)>;

#[derive(Clone)]
pub struct WideColor {
    r: f64,
    g: f64,
    b: f64,
}

impl AddAssign<Color> for WideColor {
    fn add_assign(&mut self, rhs: Color) {
        self.r += rhs.r as f64;
        self.g += rhs.g as f64;
        self.b += rhs.b as f64;
    }
}

impl AddAssign<&Color> for WideColor {
    fn add_assign(&mut self, rhs: &Color) {
        *self += *rhs;
    }
}

pub struct Point {
    pub x: usize,
    pub y: usize,
}

pub const fn u8_to_col(arr: [u8; 3]) -> Color {
    Color {
        r: arr[0],
        g: arr[1],
        b: arr[2],
    }
}

pub fn parse_hex(col: &str) -> Color {
    let css_col = col.parse::<CssColor>().unwrap_or(TRANSPARENT_BLACK);
    Color {
        r: css_col.r,
        g: css_col.g,
        b: css_col.b,
    }
}

pub fn lerp_color(from: &Color, to: &Color, progress: f64) -> Color {
    let progress_01 = progress.clamp(0.0, 1.0);
    Color {
        r: (from.r as f64 * (1.0 - progress_01) + to.r as f64 * progress_01) as u8,
        g: (from.g as f64 * (1.0 - progress_01) + to.g as f64 * progress_01) as u8,
        b: (from.b as f64 * (1.0 - progress_01) + to.b as f64 * progress_01) as u8,
    }
}

pub fn fade_into_frame(frame_to: &Frame, fade_time_ms: u32) {
    // Calculate how many steps we need to take
    let iterations = fade_time_ms / FRAME_DURATION_MS;
    // don't cause a deadlock (by later inserting into the same map), copy the starting frame
    let frame_from = KEYBOARD_LAST_FRAME.read().unwrap().clone();
    // Iterate (+1 to immediately start changing, 0 = starting frame)
    for i in 1..(iterations + 1) {
        // Add frame to the queue
        enq_keyboard_frame(
            frame_from
                .iter()
                .zip(frame_to.iter())
                .map(|(color_from, color_to)| -> Color {
                    lerp_color(color_from, color_to, i as f64 / iterations as f64)
                })
                .collect(),
        );
    }
}

pub fn get_timestamp() -> u128 {
    // Self-explanatory
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards")
        .as_millis()
}

pub fn flash_color(
    keyboard_info: &Arc<ControllerInfo>,
    color: Color,
    hold: u64,
    progress_map: &Arc<ProgressMap>,
    notifications: &Arc<RwLock<Vec<Notification>>>,
) -> bool {
    // Store the target color right away
    KEYBOARD_FLASH_COLOR.store(color, Ordering::Relaxed);
    // Animate! (300ms)
    composite(keyboard_info, progress_map, notifications, Some(300));

    tokio::spawn({
        let keyboard_info_clone = keyboard_info.clone();
        // Copy the Arc to move it into the deferred function call
        let flash_clone = KEYBOARD_FLASH_COLOR.clone();
        // Also copy Arcs for the progress map and notifications
        let progress_map_clone = progress_map.clone();
        let notifications_clone = notifications.clone();
        // Move them into the closure
        async move {
            // Wait for specified amount of time
            tokio::time::sleep(Duration::from_millis(hold)).await;
            // Store black frame (flash off)
            flash_clone.store(BLACK, Ordering::Relaxed);
            // Animate!
            composite(
                &keyboard_info_clone,
                &progress_map_clone,
                &notifications_clone,
                Some(300),
            );
        }
    });
    true
}

pub fn composite(
    keyboard_info: &ControllerInfo,
    progress_map: &ProgressMap,
    notifications_lock: &RwLock<Vec<Notification>>,
    fade_time_ms: Option<u32>,
) -> bool {
    info!("COMPOSITE !");
    // Get the contents from the RwLock
    let notifications = notifications_lock.read().unwrap();

    // This is the array that will hold colors of the loading bar at the top of the keyboard
    // Initialise it to black initially
    let mut top_bar: Vec<WideColor> = vec![
        WideColor {
            r: 0.0,
            g: 0.0,
            b: 0.0
        };
        keyboard_info.width
    ];
    // Start from the base frame
    let mut new_frame = get_keyboard_base(keyboard_info);
    // How many loading bars d we have
    let mut num_bars: usize = 0;
    // How many colored(filled) leds do we have
    let mut colored_leds: u32 = 0;

    let corrected_top_row_len =
        keyboard_info.width - KEYBOARD_COL_OFFSET_START - KEYBOARD_COL_OFFSET_END;

    for progress_tuple in progress_map {
        let color = progress_tuple.0;
        let progress = progress_tuple.1;

        // Skip if the progress is at 0
        if progress <= 0.0 {
            continue;
        }
        // This loading bar is good, increment the count
        num_bars += 1;

        // Scale to fill the entire top row
        let scaled_progress = progress * corrected_top_row_len as f64;
        // Remove the floating part
        let filled_leds = scaled_progress as usize;
        // Update the number of colored leds (take maximum)
        colored_leds = colored_leds.max(scaled_progress.ceil() as u32);

        // Calculate the progress of the last led (fade smoothly)
        let last_led_progress = scaled_progress - filled_leds as f64;

        // Add color to the bar
        (0..filled_leds).for_each(|i| {
            top_bar[i] += color;
        });
        // Lerp the last led, we can index into filled_leds because COL_OFFSET_END is 4 and top bar is always has some headroom
        // TODO: Check properly
        top_bar[filled_leds] += lerp_color(&new_frame[filled_leds], &color, last_led_progress);
    }

    // Get the flash color
    let flash = KEYBOARD_FLASH_COLOR.load(Ordering::Relaxed);

    if flash != BLACK {
        // We need to flash
        for i in 0..corrected_top_row_len {
            // Fill the top bar
            new_frame[i + KEYBOARD_COL_OFFSET_START] = flash;
        }
    } else {
        // Normalise the color
        for i in 0..colored_leds as usize {
            new_frame[i + KEYBOARD_COL_OFFSET_START] = Color {
                r: (top_bar[i].r / num_bars as f64) as u8,
                g: (top_bar[i].g / num_bars as f64) as u8,
                b: (top_bar[i].b / num_bars as f64) as u8,
            };
        }

        let mut index = KEYBOARD_COL_OFFSET_START + 2;
        for notification in notifications.iter() {
            new_frame[index] = notification.settings.color;
            index += 1;
        }
    }

    // Finally fade into the new frame
    fade_into_frame(&new_frame, fade_time_ms.unwrap_or(110));
    true
}

pub fn get_keyboard_base(keyboard_info: &ControllerInfo) -> Frame {
    if SCREEN_LOCKED.load(Ordering::Relaxed) {
        vec![DIM_GRAY; keyboard_info.total_leds]
    } else {
        KEYBOARD_BASE_FRAME.read().unwrap().clone()
    }
}

pub struct KeyMap<'a> {
    pub keys: Vec<&'a str>,
    pub color: Color,
}

// Map keyboard key names to colors
pub fn get_frame_by_key_names<'a>(
    leds: impl Iterator<Item = (usize, &'a LED)>,
    keymaps: Vec<KeyMap>,
    fallback_function: &dyn Fn(&LED, usize) -> Color,
) -> Frame {
    return leds
        .map(|(index, led)| -> Color {
            // Try to find the led in any keymap
            let mapping = keymaps.iter().find(|keymap| -> bool {
                keymap
                    .keys
                    .iter()
                    .any(|key_substr| led.name.contains(key_substr))
            });
            match mapping {
                Some(map) => map.color,
                None => fallback_function(led, index),
            }
        })
        .collect();
}
