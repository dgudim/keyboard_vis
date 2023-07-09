use core::num;
use std::{collections::HashMap, ops::AddAssign, sync::RwLock};

use concurrent_queue::ConcurrentQueue;
use css_color_parser::Color as CssColor;
use once_cell::sync::Lazy;
use openrgb::data::{Color, LED};

use crate::enq_frame;

pub type LazyColor = Lazy<Color>;

pub type Frame = Vec<Color>;
pub type LazyFrame = Lazy<Frame>;

pub struct Notification {
    pub id: u32,
    pub source: String,
    pub color: Color,
    pub timestamp: u128
}

pub type ProgressMap = HashMap<String, (Color, f64)>;

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
    pub x: u32,
    pub y: u32,
}

pub const ROW_LENGTH: u32 = 22;
pub const TOP_ROW_LENGTH: u32 = ROW_LENGTH - 4;
pub const ROWS: u32 = 6;
pub const TOTAL_LEDS: u32 = ROW_LENGTH * ROWS;

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

pub static MAIN_COLOR: LazyColor = Lazy::new(|| parse_hex("#df3b00"));
pub static TOP_ROW_COLOR: LazyColor = Lazy::new(|| parse_hex("#FFD800"));
pub static FUNCTION_COLOR: LazyColor = Lazy::new(|| parse_hex("#B200FF"));
pub static FUNCTION_COLOR2: LazyColor = Lazy::new(|| parse_hex("#B200AA"));
pub static NUM_PAD_COLOR: LazyColor = Lazy::new(|| parse_hex("#0094FF"));

pub const RED: LazyColor = Lazy::new(|| parse_hex("#ff4242"));
pub const GREEN: LazyColor = Lazy::new(|| parse_hex("#68ff42"));
pub const BLUE: LazyColor = Lazy::new(|| parse_hex("#42adff"));
pub const PURPLE: LazyColor = Lazy::new(|| parse_hex("#c342ff"));

pub static GRAY_SUBSTRATE: LazyFrame = Lazy::new(|| vec![GRAY; TOTAL_LEDS as usize]);
pub static BLACK_SUBSTRATE: LazyFrame = Lazy::new(|| vec![BLACK; TOTAL_LEDS as usize]);

pub static LAST_FRAME: Lazy<RwLock<Frame>> = Lazy::new(|| RwLock::new(BLACK_SUBSTRATE.clone()));
pub static FRAME_Q: Lazy<ConcurrentQueue<Frame>> = Lazy::new(|| ConcurrentQueue::unbounded());
pub const KEYBOARD_NAME: &str = "Razer Ornata Chroma";

pub fn xy2num(x: u32, y: u32) -> u32 {
    let xc = x.clamp(0, ROW_LENGTH - 1);
    let yc = y.clamp(0, ROWS - 1);
    return xc + (ROWS - 1 - yc) * ROW_LENGTH;
}

pub fn num2xy(n: u32) -> Point {
    let nc = n.clamp(0, TOTAL_LEDS);
    let y = nc / ROW_LENGTH;
    let x = nc - y * ROW_LENGTH;
    return Point { x, y: ROWS - y - 1 };
}

pub fn parse_hex(col: &str) -> Color {
    let css_col = col.parse::<CssColor>().unwrap_or(TRANSPARENT_BLACK);
    return Color {
        r: css_col.r,
        g: css_col.g,
        b: css_col.b,
    };
}

pub fn lerp_color(from: &Color, to: &Color, progress: f64) -> Color {
    let progress_01 = progress.clamp(0.0, 1.0);
    return Color {
        r: (from.r as f64 * (1.0 - progress_01) + to.r as f64 * progress_01) as u8,
        g: (from.g as f64 * (1.0 - progress_01) + to.g as f64 * progress_01) as u8,
        b: (from.b as f64 * (1.0 - progress_01) + to.b as f64 * progress_01) as u8,
    };
}

pub fn fade_into_frame(frame_to: &Frame, time_ms: u32) {
    let iterations = time_ms / FRAME_DELTA;
    let frame_from = LAST_FRAME
        .read()
        .expect("Failed reading last frame")
        .clone(); // dont's cause a deadlock, copy the starting frame
    for i in 1..(iterations + 1) {
        enq_frame(
            frame_from
                .iter()
                .enumerate()
                .map(|(index, color_from)| -> Color {
                    return lerp_color(color_from, &frame_to[index], i as f64 / iterations as f64);
                })
                .collect(),
        );
    }
}

pub fn display_progress(base: &Frame, map: &ProgressMap) {
    let mut bar: Vec<WideColor> = vec![
        WideColor {
            r: 0.0,
            g: 0.0,
            b: 0.0
        };
        ROW_LENGTH as usize
    ];
    let mut new_frame = base.clone();
    let mut num_bars: usize = 0;
    let mut colored_leds: u32 = 0;

    for (_, value) in map {
        let (color, progress) = value;

        if *progress <= 0.0 {
            continue;
        }

        let scaled_progress = *progress * TOP_ROW_LENGTH as f64;
        num_bars += 1;

        let filled_leds = scaled_progress as usize;
        colored_leds = colored_leds.max(scaled_progress.ceil() as u32);

        let last_led_progress = scaled_progress - filled_leds as f64;

        for i in 0..filled_leds {
            bar[i] += color;
        }
        bar[filled_leds] += lerp_color(&new_frame[filled_leds], color, last_led_progress);
    }

    for i in 0..colored_leds as usize {
        new_frame[i] = Color {
            r: (bar[i].r / num_bars as f64) as u8,
            g: (bar[i].g / num_bars as f64) as u8,
            b: (bar[i].b / num_bars as f64) as u8,
        };
    }

    fade_into_frame(&new_frame, 230);
}

pub struct KeyMap<'a> {
    pub keys: Vec<&'a str>,
    pub color: Color,
}

pub fn get_frame_by_key_names(
    leds: &Vec<LED>,
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
            return match mapping {
                Some(map) => map.color,
                None => fallback_function(&led, index),
            };
        })
        .collect();
}
