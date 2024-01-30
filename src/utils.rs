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
pub const COL_OFFSET: usize = 1;
pub const TOP_ROW_LENGTH: usize = ROW_LENGTH - 4 - COL_OFFSET;
pub const ROWS: usize = 6;
pub const TOTAL_LEDS: usize = ROW_LENGTH * ROWS;
pub const CENTER_X: i64 = (ROW_LENGTH / 2) as i64;
pub const CENTER_Y: i64 = (ROWS / 2) as i64;

pub const FRAME_DELTA: u32 = 75;

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

pub static SCREEN_LOCKED: Lazy<Arc<AtomicBool>> = Lazy::new(|| Arc::new(AtomicBool::new(false)));
pub static FLASH_COLOR: Lazy<Arc<Atomic<Color>>> =
    Lazy::new(|| Arc::new(Atomic::new(BLACK)));
pub const KEYBOARD_NAME: &str = "Razer Ornata Chroma";
pub const PROGRESS_STEP: f64 = 0.0; // minimum value for progress delta to call a recomposite

pub fn num2xy(n: usize) -> Point {
    let nc = n.clamp(0, TOTAL_LEDS);
    let y = nc / ROW_LENGTH;
    let x = nc - y * ROW_LENGTH;
    Point { x, y: ROWS - y - 1 }
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

pub fn fade_into_frame(frame_to: &Frame, time_ms: u32) {
    let iterations = time_ms / FRAME_DELTA;
    let frame_from = LAST_FRAME.read().unwrap().clone(); // dont's cause a deadlock, copy the starting frame
    for i in 1..(iterations + 1) {
        enq_frame(
            frame_from
                .iter()
                .enumerate()
                .map(|(index, color_from)| -> Color {
                    lerp_color(color_from, &frame_to[index], i as f64 / iterations as f64)
                })
                .collect(),
        );
    }
}

pub fn get_timestamp() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards")
        .as_millis()
}

pub fn flash_color(
    color: Color,
    hold: u64,
    map: &Arc<ProgressMap>,
    notifs: &Arc<RwLock<Vec<Notification>>>,
) -> bool {
    FLASH_COLOR.store(color, Ordering::Relaxed);
    composite(map, notifs, Some(300));
    let flash_clone = FLASH_COLOR.clone();
    let mapc = map.clone();
    let notifsc = notifs.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(hold)).await;
        flash_clone.store(BLACK, Ordering::Relaxed);
        composite(&mapc, &notifsc, Some(300));
    });
    true
}

pub fn composite(
    map: &ProgressMap,
    notifs_lock: &RwLock<Vec<Notification>>,
    fade_time: Option<u32>,
) -> bool {
    println!("COMPOSITE !");
    let notifs = notifs_lock.read().unwrap();

    let mut bar: Vec<WideColor> = vec![
        WideColor {
            r: 0.0,
            g: 0.0,
            b: 0.0
        };
        ROW_LENGTH
    ];
    let mut new_frame = get_base();
    let mut num_bars: usize = 0;
    let mut colored_leds: u32 = 0;

    for value in map {
        let color = value.0;
        let progress = value.1;

        if progress <= 0.0 {
            continue;
        }

        let scaled_progress = progress * TOP_ROW_LENGTH as f64;
        num_bars += 1;

        let filled_leds = scaled_progress as usize;
        colored_leds = colored_leds.max(scaled_progress.ceil() as u32);

        let last_led_progress = scaled_progress - filled_leds as f64;

        (0..filled_leds).for_each(|i| {
            bar[i] += color;
        });
        bar[filled_leds] += lerp_color(&new_frame[filled_leds], &color, last_led_progress);
    }

    let flash = FLASH_COLOR.load(Ordering::Relaxed);

    if flash != BLACK {
        for i in 0..TOP_ROW_LENGTH {
            new_frame[i + COL_OFFSET] = flash;
        }
    } else {
        for i in 0..colored_leds as usize {
            new_frame[i + COL_OFFSET] = Color {
                r: (bar[i].r / num_bars as f64) as u8,
                g: (bar[i].g / num_bars as f64) as u8,
                b: (bar[i].b / num_bars as f64) as u8,
            };
        }

        let mut index = COL_OFFSET + 2;
        for notif in notifs.iter() {
            new_frame[index] = notif.settings.color;
            index += 1;
        }
    }

    fade_into_frame(&new_frame, fade_time.unwrap_or(110));
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

pub fn get_frame_by_key_names(
    leds: &[LED],
    map: Vec<KeyMap>,
    fallback_function: &dyn Fn(&LED, usize) -> Color,
) -> Frame {
    return leds
        .iter()
        .enumerate()
        .map(|(index, led)| -> Color {
            let mapping = map.iter().find(|keymap| -> bool {
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
