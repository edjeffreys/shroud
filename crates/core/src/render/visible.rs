//! Test-only.

pub fn visible(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let mut out = String::new();
    for (i, ch) in text.char_indices() {
        match ch {
            '\x1b' => {
                if i > 0 {
                    out.push('\n');
                }
                out.push_str("\\e");
            }
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 || c == '\x7f' => out.push_str(&format!("\\x{:02x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}
