use std::sync::{atomic::{AtomicBool, AtomicU8}, Arc, RwLock};

use atomic::Atomic;
use color_hex::color_from_hex;
use concurrent_queue::ConcurrentQueue;
use once_cell::sync::Lazy;
use openrgb::data::Color;
use css_color_parser::Color as CssColor;

use crate::u8_to_col;

// Workarounds quirks in some keyboards (skip esc key, etc) this offsets the starting position of the top bar
pub const KEYBOARD_COL_OFFSET_START: usize = 1;
// The same, but fo the end of the top bar
pub const KEYBOARD_COL_OFFSET_END: usize = 4;

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

pub type Frame = Vec<Color>;

pub const MAIN_COLOR: Color = u8_to_col(color_from_hex!("#9e2000"));
pub const TOP_ROW_COLOR: Color = u8_to_col(color_from_hex!("#d19900"));
pub const FUNCTION_COLOR: Color = u8_to_col(color_from_hex!("#7800ab"));
pub const FUNCTION_COLOR2: Color = u8_to_col(color_from_hex!("#8a0084"));
pub const NUM_PAD_COLOR: Color = u8_to_col(color_from_hex!("#005da1"));

pub const BACKLIGHT_WAVE1_COLOR: Color = u8_to_col(color_from_hex!("#662a00"));
pub const BACKLIGHT_WAVE2_COLOR: Color = u8_to_col(color_from_hex!("#2a0066"));

pub const RED: Color = u8_to_col(color_from_hex!("#ff0000"));
pub const GREEN: Color = u8_to_col(color_from_hex!("#00ff00"));
pub const BLUE: Color = u8_to_col(color_from_hex!("#0000ff"));
pub const PURPLE: Color = u8_to_col(color_from_hex!("#ff00ff"));

pub static KEYBOARD_LAST_FRAME: Lazy<RwLock<Frame>> = Lazy::new(|| RwLock::new(Vec::new()));
pub static KEYBOARD_BASE_FRAME: Lazy<RwLock<Frame>> = Lazy::new(|| RwLock::new(Vec::new()));
pub static KEYBOARD_FRAME_Q: Lazy<ConcurrentQueue<Frame>> = Lazy::new(ConcurrentQueue::unbounded);

// Arc for screen lock state and flash color
pub static SCREEN_LOCKED: Lazy<Arc<AtomicBool>> = Lazy::new(|| Arc::new(AtomicBool::new(false)));
pub static ABOUT_TO_SHUTDOWN: Lazy<Arc<AtomicU8>> = Lazy::new(|| Arc::new(AtomicU8::new(0)));
pub static KEYBOARD_FLASH_COLOR: Lazy<Arc<Atomic<Color>>> = Lazy::new(|| Arc::new(Atomic::new(BLACK)));