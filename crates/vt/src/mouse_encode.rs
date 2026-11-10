//! Positions are pane-local cells; the encoder is told cells are 1×1 "pixels" so no
//! pixel geometry leaks in.

use std::ffi::c_void;
use std::ptr;

use shroud_core::input::{Mods, MouseAction, MouseButton, MouseEvent};

use crate::ffi::*;
use crate::terminal::{Terminal, encode_into};
use crate::{Error, check};

pub struct MouseEncoder {
    enc: GhosttyMouseEncoder,
    ev: GhosttyMouseEvent,
}

impl std::fmt::Debug for MouseEncoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MouseEncoder").finish_non_exhaustive()
    }
}

impl MouseEncoder {
    pub fn new() -> Result<MouseEncoder, Error> {
        let mut m = MouseEncoder { enc: ptr::null_mut(), ev: ptr::null_mut() };
        // SAFETY: out-pointers valid; Drop frees what succeeded.
        unsafe {
            check(ghostty_mouse_encoder_new(ptr::null(), &mut m.enc))?;
            check(ghostty_mouse_event_new(ptr::null(), &mut m.ev))?;
        }
        Ok(m)
    }

    pub fn encode(&mut self, term: &Terminal, ev: &MouseEvent, cols: u16, rows: u16, out: &mut Vec<u8>) {
        let size = GhosttyMouseEncoderSize {
            size: std::mem::size_of::<GhosttyMouseEncoderSize>(),
            screen_width: cols as u32,
            screen_height: rows as u32,
            cell_width: 1,
            cell_height: 1,
            padding_top: 0,
            padding_bottom: 0,
            padding_right: 0,
            padding_left: 0,
        };
        let held = ev.action == MouseAction::Motion && ev.button.is_some();
        // SAFETY: handles are live; option values match their documented types.
        unsafe {
            ghostty_mouse_encoder_set_from_terminal(self.enc, term.raw());
            ghostty_mouse_encoder_set(self.enc, GHOSTTY_MOUSE_ENCODER_OPT_SIZE, &size as *const _ as *const c_void);
            ghostty_mouse_encoder_set(
                self.enc,
                GHOSTTY_MOUSE_ENCODER_OPT_ANY_BUTTON_PRESSED,
                &held as *const _ as *const c_void,
            );
            ghostty_mouse_event_set_action(
                self.ev,
                match ev.action {
                    MouseAction::Press => GHOSTTY_MOUSE_ACTION_PRESS,
                    MouseAction::Release => GHOSTTY_MOUSE_ACTION_RELEASE,
                    MouseAction::Motion => GHOSTTY_MOUSE_ACTION_MOTION,
                },
            );
            match ev.button {
                Some(b) => {
                    ghostty_mouse_event_set_button(self.ev, button(b));
                }
                None => {
                    ghostty_mouse_event_clear_button(self.ev);
                }
            }
            let mut m = 0u32;
            if ev.mods.contains(Mods::SHIFT) {
                m |= GHOSTTY_MODS_SHIFT;
            }
            if ev.mods.contains(Mods::ALT) {
                m |= GHOSTTY_MODS_ALT;
            }
            if ev.mods.contains(Mods::CTRL) {
                m |= GHOSTTY_MODS_CTRL;
            }
            ghostty_mouse_event_set_mods(self.ev, m as GhosttyMods);
            let pos = GhosttyMousePosition { x: ev.x as f32 + 0.5, y: ev.y as f32 + 0.5 };
            ghostty_mouse_event_set_position(self.ev, pos);
            encode_into(out, |buf, len, written| ghostty_mouse_encoder_encode(self.enc, self.ev, buf, len, written));
        }
    }
}

impl Drop for MouseEncoder {
    fn drop(&mut self) {
        // SAFETY: freed once.
        unsafe {
            ghostty_mouse_event_free(self.ev);
            ghostty_mouse_encoder_free(self.enc);
        }
    }
}

fn button(b: MouseButton) -> GhosttyMouseButton {
    match b {
        MouseButton::Left => GHOSTTY_MOUSE_BUTTON_LEFT,
        MouseButton::Middle => GHOSTTY_MOUSE_BUTTON_MIDDLE,
        MouseButton::Right => GHOSTTY_MOUSE_BUTTON_RIGHT,
        MouseButton::WheelUp => GHOSTTY_MOUSE_BUTTON_FOUR,
        MouseButton::WheelDown => GHOSTTY_MOUSE_BUTTON_FIVE,
        MouseButton::WheelLeft => GHOSTTY_MOUSE_BUTTON_SIX,
        MouseButton::WheelRight => GHOSTTY_MOUSE_BUTTON_SEVEN,
        MouseButton::Other(_) => GHOSTTY_MOUSE_BUTTON_UNKNOWN,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sgr_click_only_when_the_program_asked() {
        let mut t = Terminal::new(80, 24).unwrap();
        let click =
            MouseEvent { action: MouseAction::Press, button: Some(MouseButton::Left), x: 4, y: 2, mods: Mods::NONE };
        let mut enc = MouseEncoder::new().unwrap();
        let mut out = Vec::new();
        enc.encode(&t, &click, 80, 24, &mut out);
        assert!(out.is_empty());
        t.write(b"\x1b[?1000h\x1b[?1006h");
        enc.encode(&t, &click, 80, 24, &mut out);
        assert_eq!(out, b"\x1b[<0;5;3M");
    }
}
