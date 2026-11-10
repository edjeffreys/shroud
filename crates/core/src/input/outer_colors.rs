//! The outer terminal's colours become pane defaults, so programs' OSC 10/11/4 queries
//! answer truthfully. Never used to recolour pane content.

pub type Rgb = [u8; 3];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ColorSlot {
    Foreground,
    Background,
    Palette(u8),
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct OuterColors {
    pub foreground: Option<Rgb>,
    pub background: Option<Rgb>,
    palette: [Option<Rgb>; 256],
}

impl Default for OuterColors {
    fn default() -> OuterColors {
        OuterColors { foreground: None, background: None, palette: [None; 256] }
    }
}

impl OuterColors {
    /// One OSC 4 asks for the whole palette; terminals answer each index separately.
    pub fn query() -> Vec<u8> {
        let mut q = b"\x1b]10;?\x1b\\\x1b]11;?\x1b\\\x1b]4".to_vec();
        for i in 0..=255u8 {
            q.extend_from_slice(format!(";{i};?").as_bytes());
        }
        q.extend_from_slice(b"\x1b\\");
        q
    }

    pub fn set(&mut self, slot: ColorSlot, rgb: Rgb) -> bool {
        let entry = match slot {
            ColorSlot::Foreground => &mut self.foreground,
            ColorSlot::Background => &mut self.background,
            ColorSlot::Palette(i) => &mut self.palette[usize::from(i)],
        };
        entry.replace(rgb) != Some(rgb)
    }

    // Setting a palette replaces all 256 defaults, so a partial one is useless.
    pub fn palette(&self) -> Option<[Rgb; 256]> {
        let mut out = [[0; 3]; 256];
        for (o, p) in out.iter_mut().zip(&self.palette) {
            *o = (*p)?;
        }
        Some(out)
    }
}

pub fn parse_report(body: &[u8]) -> Option<(ColorSlot, Rgb)> {
    let text = std::str::from_utf8(body).ok()?;
    let mut parts = text.split(';');
    let slot = match parts.next()? {
        "10" => ColorSlot::Foreground,
        "11" => ColorSlot::Background,
        "4" => ColorSlot::Palette(parts.next()?.parse().ok()?),
        _ => return None,
    };
    let rgb = parse_rgb(parts.next()?)?;
    parts.next().is_none().then_some((slot, rgb))
}

fn parse_rgb(spec: &str) -> Option<Rgb> {
    let mut chans = spec.strip_prefix("rgb:")?.split('/');
    let mut out = [0u8; 3];
    for o in &mut out {
        let h = chans.next()?;
        if h.is_empty() || h.len() > 4 {
            return None;
        }
        let v = u32::from_str_radix(h, 16).ok()?;
        let max = (1u32 << (4 * h.len())) - 1;
        *o = ((v * 255 + max / 2) / max) as u8;
    }
    chans.next().is_none().then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_parse_at_every_channel_width() {
        assert_eq!(parse_report(b"11;rgb:1e1e/1e1e/2e2e"), Some((ColorSlot::Background, [0x1e, 0x1e, 0x2e])));
        assert_eq!(parse_report(b"10;rgb:f/8/0"), Some((ColorSlot::Foreground, [255, 136, 0])));
        assert_eq!(parse_report(b"4;12;rgb:00/80/ff"), Some((ColorSlot::Palette(12), [0, 0x80, 0xff])));
    }

    #[test]
    fn junk_is_not_a_report() {
        for bad in [
            &b"11;?"[..],
            b"4;256;rgb:0/0/0",
            b"12;rgb:0/0/0",
            b"11;rgb:0/0",
            b"11;rgb:00000/0/0",
            b"11;rgb:g/0/0",
            b"2;title",
            b"\xff",
        ] {
            assert_eq!(parse_report(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn palette_needs_every_index() {
        let mut c = OuterColors::default();
        for i in 0..255u8 {
            c.set(ColorSlot::Palette(i), [i, 0, 0]);
        }
        assert!(c.palette().is_none());
        assert!(c.set(ColorSlot::Palette(255), [1, 2, 3]));
        assert!(!c.set(ColorSlot::Palette(255), [1, 2, 3]));
        assert_eq!(c.palette().unwrap()[255], [1, 2, 3]);
    }

    #[test]
    fn query_asks_for_all_slots() {
        let q = String::from_utf8(OuterColors::query()).unwrap();
        assert!(q.starts_with("\x1b]10;?\x1b\\\x1b]11;?\x1b\\\x1b]4;0;?;1;?"));
        assert!(q.ends_with(";255;?\x1b\\"));
    }
}
