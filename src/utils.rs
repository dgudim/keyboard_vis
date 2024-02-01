use std::{
    ops::AddAssign,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, RwLock,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use atomic::Atomic;
use color_hex::color_from_hex;
use concurrent_queue::ConcurrentQueue;
use css_color_parser::Color as CssColor;
use dashmap::DashMap;
use log::info;
use once_cell::sync::Lazy;
use openrgb::data::{Color, LED};

use crate::enq_frame;

pub type Frame = Vec<Color>;
pub type LazyFrame = Lazy<Frame>;

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

pub const ROW_LENGTH: usize = 22;
pub const ROW_COUNT: usize = 6;
pub const TOTAL_LEDS: usize = ROW_LENGTH * ROW_COUNT;

// Workarounds quirks in some keyboards (skip esc key, etc) this offsets the starting position of the top bar
pub const COL_OFFSET_START: usize = 1;
// The same, but fo the end of the top bar
pub const COL_OFFSET_END: usize = 4;
pub const TOP_ROW_LENGTH: usize = ROW_LENGTH - COL_OFFSET_END - COL_OFFSET_START;

pub const CENTER_X: i64 = (ROW_LENGTH / 2) as i64;
pub const CENTER_Y: i64 = (ROW_COUNT / 2) as i64;

// How many ms per frame
pub const FRAME_DURATION_MS: u32 = 75;

// Define some constants (colors)
pub const TRANSPARENT_BLACK: CssColor = CssColor {
    r: 0,
    g: 0,
    b: 0,
    a: 1.0,
};

pub const BLACK: Color = Color { r: 0, g: 0, b: 0 };
pub const WHITE: Color = Color {
    r: 255,
    g: 255,
    b: 255,
};
pub const GRAY: Color = Color {
    r: 80,
    g: 65,
    b: 80,
};
pub const DIM_GRAY: Color = Color {
    r: 40,
    g: 35,
    b: 40,
};

pub const MAIN_COLOR: Color = u8_to_col(color_from_hex!("#9e2000"));
pub const TOP_ROW_COLOR: Color = u8_to_col(color_from_hex!("#d19900"));
pub const FUNCTION_COLOR: Color = u8_to_col(color_from_hex!("#7800ab"));
pub const FUNCTION_COLOR2: Color = u8_to_col(color_from_hex!("#8a0084"));
pub const NUM_PAD_COLOR: Color = u8_to_col(color_from_hex!("#005da1"));

pub const RED: Color = u8_to_col(color_from_hex!("#ff0000"));
pub const GREEN: Color = u8_to_col(color_from_hex!("#00ff00"));
pub const BLUE: Color = u8_to_col(color_from_hex!("#0000ff"));
pub const PURPLE: Color = u8_to_col(color_from_hex!("#ff00ff"));

pub static GRAY_SUBSTRATE: LazyFrame = Lazy::new(|| vec![GRAY; TOTAL_LEDS]);
pub static DIM_GRAY_SUBSTRATE: LazyFrame = Lazy::new(|| vec![DIM_GRAY; TOTAL_LEDS]);
pub static BLACK_SUBSTRATE: LazyFrame = Lazy::new(|| vec![BLACK; TOTAL_LEDS]);

pub static LAST_FRAME: Lazy<RwLock<Frame>> = Lazy::new(|| RwLock::new(BLACK_SUBSTRATE.clone()));
pub static BASE_FRAME: Lazy<RwLock<Frame>> = Lazy::new(|| RwLock::new(BLACK_SUBSTRATE.clone()));
pub static FRAME_Q: Lazy<ConcurrentQueue<Frame>> = Lazy::new(ConcurrentQueue::unbounded);

// Arc for screen lock state and flash color
pub static SCREEN_LOCKED: Lazy<Arc<AtomicBool>> = Lazy::new(|| Arc::new(AtomicBool::new(false)));
pub static FLASH_COLOR: Lazy<Arc<Atomic<Color>>> = Lazy::new(|| Arc::new(Atomic::new(BLACK)));
// TODO: This should be in a config somewhere
pub const KEYBOARD_NAME: &str = "Razer Ornata Chroma";
// Minimum value for progress delta to cause a recomposite
pub const PROGRESS_STEP: f64 = 0.0;

// Index of the led into xy coordinates
pub fn num2xy(n: usize) -> Point {
    let nc = n.clamp(0, TOTAL_LEDS);
    let y = nc / ROW_LENGTH;
    let x = nc - y * ROW_LENGTH;
    Point {
        x,
        y: ROW_COUNT - y - 1,
    }
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
    let frame_from = LAST_FRAME.read().unwrap().clone();
    // Iterate (+1 to immediately start changing, 0 = starting frame)
    for i in 1..(iterations + 1) {
        // Add frame to the queue
        enq_frame(
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
    color: Color,
    hold: u64,
    progress_map: &Arc<ProgressMap>,
    notifications: &Arc<RwLock<Vec<Notification>>>,
) -> bool {
    // Store the target color right away
    FLASH_COLOR.store(color, Ordering::Relaxed);
    // Animate! (300ms)
    composite(progress_map, notifications, Some(300));

    tokio::spawn({
        // Copy the Arc to move it into the deferred function call
        let flash_clone = FLASH_COLOR.clone();
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
            composite(&progress_map_clone, &notifications_clone, Some(300));
        }
    });
    true
}

pub fn composite(
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
        ROW_LENGTH
    ];
    // Start from the base frame
    let mut new_frame = get_base();
    // How many loading bars d we have
    let mut num_bars: usize = 0;
    // How many colored(filled) leds do we have
    let mut colored_leds: u32 = 0;

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
        let scaled_progress = progress * TOP_ROW_LENGTH as f64;
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
    let flash = FLASH_COLOR.load(Ordering::Relaxed);

    if flash != BLACK {
        // We need to flash
        for i in 0..TOP_ROW_LENGTH {
            // Fill the top bar
            new_frame[i + COL_OFFSET_START] = flash;
        }
    } else {
        // Normalise the color
        for i in 0..colored_leds as usize {
            new_frame[i + COL_OFFSET_START] = Color {
                r: (top_bar[i].r / num_bars as f64) as u8,
                g: (top_bar[i].g / num_bars as f64) as u8,
                b: (top_bar[i].b / num_bars as f64) as u8,
            };
        }

        let mut index = COL_OFFSET_START + 2;
        for notification in notifications.iter() {
            new_frame[index] = notification.settings.color;
            index += 1;
        }
    }

    // Finally fade into the new frame
    fade_into_frame(&new_frame, fade_time_ms.unwrap_or(110));
    true
}

pub fn get_base() -> Frame {
    if SCREEN_LOCKED.load(Ordering::Relaxed) {
        DIM_GRAY_SUBSTRATE.clone()
    } else {
        BASE_FRAME.read().unwrap().clone()
    }
}

pub struct KeyMap<'a> {
    pub keys: Vec<&'a str>,
    pub color: Color,
}

// Map keyboard key names to colors
pub fn get_frame_by_key_names(
    leds: &[LED],
    keymaps: Vec<KeyMap>,
    fallback_function: &dyn Fn(&LED, usize) -> Color,
) -> Frame {
    return leds
        .iter()
        .enumerate()
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
