//! libghostty-vt callbacks run mid-`write` and the library isn't re-entrant, so they
//! only push to an effect queue the caller drains afterwards.

use std::ffi::c_void;
use std::ptr;

use shroud_core::shell::Mark;

use crate::ffi::*;
use crate::{Error, check};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Replies to queries (DA, DSR, mode reports); must be written to the PTY in order.
    WritePty(Vec<u8>),
    Bell,
    Title(String),
    Pwd(String),
    Clipboard(Vec<u8>),
    Notification {
        title: String,
        body: String,
    },
    Shell(Mark),
    RenderHold(bool),
}

struct Inner {
    effects: Vec<Effect>,
    size: GhosttySizeReportSize,
    dark: Option<bool>,
}

pub struct Terminal {
    pub(crate) raw: GhosttyTerminal,
    // Raw, not Box: callbacks write through this pointer while `&mut self` is live.
    inner: *mut Inner,
}

impl std::fmt::Debug for Terminal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Terminal").finish_non_exhaustive()
    }
}

impl Terminal {
    pub fn new(cols: u16, rows: u16) -> Result<Terminal, Error> {
        let mut raw: GhosttyTerminal = ptr::null_mut();
        // SAFETY: out-pointer is valid; a null allocator selects the library default.
        check(unsafe { ghostty_terminal_new(ptr::null(), &mut raw, cols, rows) })?;
        let size = GhosttySizeReportSize { rows, columns: cols, cell_width: 0, cell_height: 0 };
        let inner = Box::into_raw(Box::new(Inner { effects: Vec::new(), size, dark: None }));
        let t = Terminal { raw, inner };
        t.set(GHOSTTY_TERMINAL_OPT_USERDATA, inner as *const c_void)?;
        t.set_fn(GHOSTTY_TERMINAL_OPT_WRITE_PTY, on_write_pty as *const c_void)?;
        t.set_fn(GHOSTTY_TERMINAL_OPT_BELL, on_bell as *const c_void)?;
        t.set_fn(GHOSTTY_TERMINAL_OPT_TITLE_CHANGED, on_title as *const c_void)?;
        t.set_fn(GHOSTTY_TERMINAL_OPT_PWD_CHANGED, on_pwd as *const c_void)?;
        t.set_fn(GHOSTTY_TERMINAL_OPT_CLIPBOARD_WRITE, on_clipboard_write as *const c_void)?;
        t.set_fn(GHOSTTY_TERMINAL_OPT_DESKTOP_NOTIFICATION, on_notification as *const c_void)?;
        t.set_fn(GHOSTTY_TERMINAL_OPT_SEMANTIC_PROMPT, on_semantic_prompt as *const c_void)?;
        t.set_fn(GHOSTTY_TERMINAL_OPT_RENDER_HOLD, on_render_hold as *const c_void)?;
        t.set_fn(GHOSTTY_TERMINAL_OPT_SIZE, on_size as *const c_void)?;
        t.set_fn(GHOSTTY_TERMINAL_OPT_COLOR_SCHEME, on_color_scheme as *const c_void)?;
        t.allow_image_media();
        Ok(t)
    }

    pub fn write(&mut self, bytes: &[u8]) {
        // SAFETY: raw is live; callbacks only touch `inner`, which outlives this call.
        unsafe { ghostty_terminal_vt_write(self.raw, bytes.as_ptr(), bytes.len()) }
    }

    pub fn drain_effects(&mut self, out: &mut Vec<Effect>) {
        // SAFETY: no C call is in progress, so nothing else references inner.
        out.append(unsafe { &mut (*self.inner).effects });
    }

    pub fn resize(&mut self, cols: u16, rows: u16, cell_w: u32, cell_h: u32) -> Result<(), Error> {
        unsafe {
            (*self.inner).size = GhosttySizeReportSize { rows, columns: cols, cell_width: cell_w, cell_height: cell_h };
            check(ghostty_terminal_resize(self.raw, cols, rows, cell_w, cell_h))
        }
    }

    pub fn set_color_scheme(&mut self, dark: Option<bool>) {
        unsafe { (*self.inner).dark = dark }
    }

    /// `None` unsets: the terminal then stays silent on OSC 10/11 and uses its built-in palette.
    pub fn set_default_colors(&mut self, fg: Option<[u8; 3]>, bg: Option<[u8; 3]>, palette: Option<&[[u8; 3]; 256]>) {
        let rgb = |[r, g, b]: [u8; 3]| GhosttyColorRgb { r, g, b };
        let one = |c: Option<[u8; 3]>| c.map(rgb);
        let (fg, bg) = (one(fg), one(bg));
        let pal = palette.map(|p| p.map(rgb));
        let ptr = |c: &Option<GhosttyColorRgb>| c.as_ref().map_or(ptr::null(), |c| c as *const _ as *const c_void);
        let _ = self.set(GHOSTTY_TERMINAL_OPT_COLOR_FOREGROUND, ptr(&fg));
        let _ = self.set(GHOSTTY_TERMINAL_OPT_COLOR_BACKGROUND, ptr(&bg));
        let _ = self
            .set(GHOSTTY_TERMINAL_OPT_COLOR_PALETTE, pal.as_ref().map_or(ptr::null(), |p| p.as_ptr() as *const c_void));
    }

    pub fn hyperlink_at(&self, x: u16, y: u16) -> Option<String> {
        let point = GhosttyPoint {
            tag: GHOSTTY_POINT_TAG_VIEWPORT,
            value: GhosttyPointValue { coordinate: GhosttyPointCoordinate { x, y: u32::from(y) } },
        };
        let mut r: GhosttyGridRef = unsafe { std::mem::zeroed() };
        // SAFETY: out-pointers are valid; a zeroed ref is the documented starting value.
        unsafe {
            if ghostty_terminal_grid_ref(self.raw, point, &mut r) != GHOSTTY_SUCCESS {
                return None;
            }
            let mut len = 0usize;
            let _ = ghostty_grid_ref_hyperlink_uri(&r, ptr::null_mut(), 0, &mut len);
            if len == 0 {
                return None;
            }
            let mut buf = vec![0u8; len];
            check(ghostty_grid_ref_hyperlink_uri(&r, buf.as_mut_ptr(), len, &mut len)).ok()?;
            buf.truncate(len);
            String::from_utf8(buf).ok()
        }
    }

    pub fn title(&self) -> String {
        self.get_string(GHOSTTY_TERMINAL_DATA_TITLE)
    }

    pub fn mode(&self, value: u16, ansi: bool) -> bool {
        let mut cfg = GhosttyTerminalModeConfig { mode: (value & 0x7fff) | (u16::from(ansi) << 15), value: false };
        let r = self.get(GHOSTTY_TERMINAL_DATA_MODE, &mut cfg as *mut _ as *mut c_void);
        r.is_ok() && cfg.value
    }

    /// Also the value a program's reset returns to.
    pub fn set_mode_default(&mut self, value: u16, ansi: bool, on: bool) {
        let cfg = GhosttyTerminalModeConfig { mode: (value & 0x7fff) | (u16::from(ansi) << 15), value: on };
        let _ = self.set(GHOSTTY_TERMINAL_OPT_MODE_DEFAULT, &cfg as *const _ as *const c_void);
    }

    pub fn mouse_tracking(&self) -> bool {
        let mut on = false;
        self.get(GHOSTTY_TERMINAL_DATA_MOUSE_TRACKING, &mut on as *mut _ as *mut c_void).is_ok() && on
    }

    /// Positive scrolls toward the bottom.
    pub fn scroll_viewport(&mut self, delta: isize) {
        let sv = GhosttyTerminalScrollViewport {
            tag: GHOSTTY_TERMINAL_SCROLL_VIEWPORT_DELTA,
            value: GhosttyTerminalScrollViewportValue { delta },
        };
        let _ = unsafe { ghostty_terminal_scroll_viewport(self.raw, sv) };
    }

    pub fn scroll_to_bottom(&mut self) {
        let sv = GhosttyTerminalScrollViewport {
            tag: GHOSTTY_TERMINAL_SCROLL_VIEWPORT_BOTTOM,
            value: GhosttyTerminalScrollViewportValue { delta: 0 },
        };
        let _ = unsafe { ghostty_terminal_scroll_viewport(self.raw, sv) };
    }

    pub fn encode_paste(&self, data: &[u8], out: &mut Vec<u8>) {
        let bracketed = self.mode(2004, false);
        let mut copy = data.to_vec();
        encode_into(out, |buf, len, written| {
            // SAFETY: `copy` is a mutable scratch buffer the encoder may sanitise in place.
            unsafe { ghostty_paste_encode(copy.as_mut_ptr().cast(), copy.len(), bracketed, buf, len, written) }
        });
    }

    pub fn encode_focus(&self, gained: bool, out: &mut Vec<u8>) {
        if !self.mode(1004, false) {
            return;
        }
        let ev = if gained { GHOSTTY_FOCUS_GAINED } else { GHOSTTY_FOCUS_LOST };
        encode_into(out, |buf, len, written| unsafe { ghostty_focus_encode(ev, buf, len, written) });
    }

    pub(crate) fn raw(&self) -> GhosttyTerminal {
        self.raw
    }

    pub(crate) fn set(&self, opt: GhosttyTerminalOption, value: *const c_void) -> Result<(), Error> {
        // SAFETY: each option's value type matches what the caller passes.
        check(unsafe { ghostty_terminal_set(self.raw, opt, value) })
    }

    fn set_fn(&self, opt: GhosttyTerminalOption, f: *const c_void) -> Result<(), Error> {
        self.set(opt, f)
    }

    pub(crate) fn get(&self, data: GhosttyTerminalData, out: *mut c_void) -> Result<(), Error> {
        // SAFETY: out points to the type documented for `data`.
        check(unsafe { ghostty_terminal_get(self.raw, data, out) })
    }

    fn get_string(&self, data: GhosttyTerminalData) -> String {
        get_string(self.raw, data)
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        // SAFETY: freed exactly once; no callbacks can run after terminal_free.
        unsafe {
            ghostty_terminal_free(self.raw);
            drop(Box::from_raw(self.inner));
        }
    }
}

pub(crate) fn encode_into(out: &mut Vec<u8>, mut f: impl FnMut(*mut u8, usize, *mut usize) -> GhosttyResult) {
    let mut buf = [0u8; 128];
    let mut written = 0usize;
    match f(buf.as_mut_ptr(), buf.len(), &mut written) {
        GHOSTTY_SUCCESS => out.extend_from_slice(&buf[..written]),
        GHOSTTY_OUT_OF_SPACE => {
            let mut big = vec![0u8; written];
            if f(big.as_mut_ptr(), big.len(), &mut written) == GHOSTTY_SUCCESS {
                out.extend_from_slice(&big[..written]);
            }
        }
        _ => {}
    }
}

fn get_string(raw: GhosttyTerminal, data: GhosttyTerminalData) -> String {
    let mut s = GhosttyString { ptr: ptr::null(), len: 0 };
    // SAFETY: GhosttyString is the documented output type for TITLE and PWD.
    let ok = unsafe { ghostty_terminal_get(raw, data, &mut s as *mut _ as *mut c_void) } == GHOSTTY_SUCCESS;
    if ok { string(s) } else { String::new() }
}

fn string(s: GhosttyString) -> String {
    if s.ptr.is_null() || s.len == 0 {
        return String::new();
    }
    // SAFETY: the library guarantees ptr/len describe a live buffer for the call's duration.
    String::from_utf8_lossy(unsafe { std::slice::from_raw_parts(s.ptr, s.len) }).into_owned()
}

/// # Safety
/// `ud` must be the `Inner` pointer registered as userdata.
unsafe fn inner<'a>(ud: *mut c_void) -> &'a mut Inner {
    unsafe { &mut *(ud as *mut Inner) }
}

unsafe extern "C" fn on_write_pty(_t: GhosttyTerminal, ud: *mut c_void, data: *const u8, len: usize) {
    let bytes = unsafe { std::slice::from_raw_parts(data, len) }.to_vec();
    unsafe { inner(ud) }.effects.push(Effect::WritePty(bytes));
}

unsafe extern "C" fn on_bell(_t: GhosttyTerminal, ud: *mut c_void) {
    unsafe { inner(ud) }.effects.push(Effect::Bell);
}

unsafe extern "C" fn on_title(t: GhosttyTerminal, ud: *mut c_void) {
    let title = get_string(t, GHOSTTY_TERMINAL_DATA_TITLE);
    unsafe { inner(ud) }.effects.push(Effect::Title(title));
}

unsafe extern "C" fn on_pwd(t: GhosttyTerminal, ud: *mut c_void) {
    let pwd = get_string(t, GHOSTTY_TERMINAL_DATA_PWD);
    unsafe { inner(ud) }.effects.push(Effect::Pwd(pwd));
}

unsafe extern "C" fn on_clipboard_write(_t: GhosttyTerminal, ud: *mut c_void, write: *const GhosttyClipboardWrite) {
    let w = unsafe { &*write };
    let contents =
        if w.contents.is_null() { &[][..] } else { unsafe { std::slice::from_raw_parts(w.contents, w.contents_len) } };
    let text = contents.iter().find(|c| string(c.mime).starts_with("text/plain")).or(contents.first());
    let data = text.map(|c| string_bytes(c.data)).unwrap_or_default();
    unsafe { inner(ud) }.effects.push(Effect::Clipboard(data));
    if let Some(reply) = w.reply {
        let r = GhosttyClipboardWriteReply {
            size: std::mem::size_of::<GhosttyClipboardWriteReply>(),
            result: GHOSTTY_CLIPBOARD_WRITE_RESULT_SUCCESS,
            remember: false,
        };
        unsafe { reply(write, &r) };
    }
}

fn string_bytes(s: GhosttyString) -> Vec<u8> {
    if s.ptr.is_null() {
        return Vec::new();
    }
    unsafe { std::slice::from_raw_parts(s.ptr, s.len) }.to_vec()
}

unsafe extern "C" fn on_notification(
    _t: GhosttyTerminal,
    ud: *mut c_void,
    n: *const GhosttyTerminalDesktopNotification,
) {
    let n = unsafe { &*n };
    let effect = Effect::Notification { title: string(n.title), body: string(n.body) };
    unsafe { inner(ud) }.effects.push(effect);
}

unsafe extern "C" fn on_semantic_prompt(
    _t: GhosttyTerminal,
    ud: *mut c_void,
    ev: *const GhosttyTerminalSemanticPrompt,
) {
    let ev = unsafe { &*ev };
    let mark = match ev.kind {
        GHOSTTY_SEMANTIC_PROMPT_PROMPT_START => Mark::PromptStart,
        GHOSTTY_SEMANTIC_PROMPT_INPUT_START => Mark::InputStart,
        GHOSTTY_SEMANTIC_PROMPT_OUTPUT_START => Mark::OutputStart,
        GHOSTTY_SEMANTIC_PROMPT_COMMAND_END => Mark::CommandEnd { exit: ev.has_exit_code.then_some(ev.exit_code) },
        _ => return,
    };
    unsafe { inner(ud) }.effects.push(Effect::Shell(mark));
}

unsafe extern "C" fn on_render_hold(_t: GhosttyTerminal, ud: *mut c_void, held: bool) {
    unsafe { inner(ud) }.effects.push(Effect::RenderHold(held));
}

unsafe extern "C" fn on_size(_t: GhosttyTerminal, ud: *mut c_void, out: *mut GhosttySizeReportSize) -> bool {
    let size = unsafe { inner(ud) }.size;
    if size.cell_width == 0 {
        return false;
    }
    unsafe { *out = size };
    true
}

unsafe extern "C" fn on_color_scheme(_t: GhosttyTerminal, ud: *mut c_void, out: *mut GhosttyColorScheme) -> bool {
    match unsafe { inner(ud) }.dark {
        Some(dark) => {
            unsafe { *out = if dark { GHOSTTY_COLOR_SCHEME_DARK } else { GHOSTTY_COLOR_SCHEME_LIGHT } };
            true
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_attributes_query_is_answered() {
        let mut t = Terminal::new(80, 24).unwrap();
        t.write(b"\x1b[c");
        let mut fx = Vec::new();
        t.drain_effects(&mut fx);
        assert!(matches!(&fx[..], [Effect::WritePty(r)] if r.starts_with(b"\x1b[?")));
    }

    #[test]
    fn title_osc52_and_osc133_become_effects() {
        let mut t = Terminal::new(80, 24).unwrap();
        t.write(b"\x1b]2;hello\x07\x1b]52;c;aGk=\x07\x1b]133;D;3\x07");
        let mut fx = Vec::new();
        t.drain_effects(&mut fx);
        assert!(fx.contains(&Effect::Title("hello".into())), "{fx:?}");
        assert!(fx.contains(&Effect::Clipboard(b"hi".to_vec())), "{fx:?}");
        assert!(fx.contains(&Effect::Shell(Mark::CommandEnd { exit: Some(3) })), "{fx:?}");
    }

    #[test]
    fn default_colours_answer_osc_queries() {
        let mut t = Terminal::new(80, 24).unwrap();
        let mut palette = [[0u8; 3]; 256];
        palette[1] = [0xab, 0, 0];
        t.set_default_colors(None, Some([0x10, 0x20, 0x30]), Some(&palette));
        t.write(b"\x1b]11;?\x1b\\\x1b]4;1;?\x1b\\");
        let mut fx = Vec::new();
        t.drain_effects(&mut fx);
        let replies: Vec<u8> = fx
            .iter()
            .filter_map(|e| if let Effect::WritePty(r) = e { Some(r.clone()) } else { None })
            .flatten()
            .collect();
        let replies = String::from_utf8_lossy(&replies);
        assert!(replies.contains("11;rgb:1010/2020/3030"), "{replies:?}");
        assert!(replies.contains("4;1;rgb:abab/0000/0000"), "{replies:?}");
    }

    #[test]
    fn modes_are_readable() {
        let mut t = Terminal::new(80, 24).unwrap();
        assert!(!t.mode(2004, false));
        t.write(b"\x1b[?2004h");
        assert!(t.mode(2004, false));
        let mut out = Vec::new();
        t.encode_paste(b"x", &mut out);
        assert_eq!(out, b"\x1b[200~x\x1b[201~");
    }
}
