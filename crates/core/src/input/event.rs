//! Key codes are kitty-protocol codes; legacy input is normalised into them too.

use super::outer_colors::{ColorSlot, Rgb};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Hash)]
pub struct Mods(pub u8);

impl Mods {
    pub const NONE: Mods = Mods(0);
    pub const SHIFT: Mods = Mods(1);
    pub const ALT: Mods = Mods(2);
    pub const CTRL: Mods = Mods(4);
    pub const SUPER: Mods = Mods(8);
    pub const HYPER: Mods = Mods(16);
    pub const META: Mods = Mods(32);
    pub const CAPS_LOCK: Mods = Mods(64);
    pub const NUM_LOCK: Mods = Mods(128);

    pub fn contains(self, other: Mods) -> bool {
        self.0 & other.0 == other.0
    }

    /// Lock keys never change what a binding means.
    pub fn without_locks(self) -> Mods {
        Mods(self.0 & !(Mods::CAPS_LOCK.0 | Mods::NUM_LOCK.0))
    }
}

impl std::ops::BitOr for Mods {
    type Output = Mods;
    fn bitor(self, rhs: Mods) -> Mods {
        Mods(self.0 | rhs.0)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum KeyAction {
    #[default]
    Press,
    Repeat,
    Release,
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct KeyEvent {
    pub code: u32,
    pub shifted: Option<u32>,
    /// The key on a US layout, for layout-independent shortcuts.
    pub base: Option<u32>,
    pub mods: Mods,
    pub action: KeyAction,
    pub text: String,
}

impl KeyEvent {
    pub fn press(code: u32, mods: Mods) -> KeyEvent {
        KeyEvent { code, mods, ..KeyEvent::default() }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    WheelUp,
    WheelDown,
    WheelLeft,
    WheelRight,
    Other(u8),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MouseAction {
    Press,
    Release,
    Motion,
}

/// Coordinates are zero-based cells on the outer terminal.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct MouseEvent {
    pub action: MouseAction,
    pub button: Option<MouseButton>,
    pub x: u16,
    pub y: u16,
    pub mods: Mods,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Event {
    Key(KeyEvent),
    Mouse(MouseEvent),
    Paste(Vec<u8>),
    Focus(bool),
    ColorScheme {
        dark: bool,
    },
    OuterColor {
        slot: ColorSlot,
        rgb: Rgb,
    },
    /// The outer terminal's DECRQM answer for mode 2027.
    GraphemeClustering(bool),
}

pub mod key {
    pub const ESCAPE: u32 = 27;
    pub const ENTER: u32 = 13;
    pub const TAB: u32 = 9;
    pub const BACKSPACE: u32 = 127;
    pub const SPACE: u32 = 32;
    pub const INSERT: u32 = 57348;
    pub const DELETE: u32 = 57349;
    pub const LEFT: u32 = 57350;
    pub const RIGHT: u32 = 57351;
    pub const UP: u32 = 57352;
    pub const DOWN: u32 = 57353;
    pub const PAGE_UP: u32 = 57354;
    pub const PAGE_DOWN: u32 = 57355;
    pub const HOME: u32 = 57356;
    pub const END: u32 = 57357;
    pub const CAPS_LOCK: u32 = 57358;
    pub const SCROLL_LOCK: u32 = 57359;
    pub const NUM_LOCK: u32 = 57360;
    pub const PRINT_SCREEN: u32 = 57361;
    pub const PAUSE: u32 = 57362;
    pub const MENU: u32 = 57363;
    pub const F1: u32 = 57364;
    pub const F35: u32 = 57398;
    pub const KP_0: u32 = 57399;
    pub const KP_9: u32 = 57408;
    pub const KP_DECIMAL: u32 = 57409;
    pub const KP_DIVIDE: u32 = 57410;
    pub const KP_MULTIPLY: u32 = 57411;
    pub const KP_SUBTRACT: u32 = 57412;
    pub const KP_ADD: u32 = 57413;
    pub const KP_ENTER: u32 = 57414;
    pub const KP_EQUAL: u32 = 57415;
    pub const KP_SEPARATOR: u32 = 57416;
    pub const KP_LEFT: u32 = 57417;
    pub const KP_RIGHT: u32 = 57418;
    pub const KP_UP: u32 = 57419;
    pub const KP_DOWN: u32 = 57420;
    pub const KP_PAGE_UP: u32 = 57421;
    pub const KP_PAGE_DOWN: u32 = 57422;
    pub const KP_HOME: u32 = 57423;
    pub const KP_END: u32 = 57424;
    pub const KP_INSERT: u32 = 57425;
    pub const KP_DELETE: u32 = 57426;
    pub const KP_BEGIN: u32 = 57427;
    pub const LEFT_SHIFT: u32 = 57441;
    pub const LEFT_CONTROL: u32 = 57442;
    pub const LEFT_ALT: u32 = 57443;
    pub const LEFT_SUPER: u32 = 57444;
    pub const RIGHT_SHIFT: u32 = 57447;
    pub const RIGHT_CONTROL: u32 = 57448;
    pub const RIGHT_ALT: u32 = 57449;
    pub const RIGHT_SUPER: u32 = 57450;

    pub fn f(n: u32) -> u32 {
        F1 + n - 1
    }
}
