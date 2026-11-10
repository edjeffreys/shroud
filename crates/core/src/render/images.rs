//! Every command carries `q=2`: a reply would reach the client as typed keys.

use crate::layout::rect::Rect;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Placement {
    pub image: u32,
    pub generation: u64,
    pub z: i32,
    pub col: i32,
    pub row: i32,
    pub offset: (u32, u32),
    pub size: (u32, u32),
    pub source: (u32, u32, u32, u32),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Placed {
    pub x: u16,
    pub y: u16,
    pub offset: (u32, u32),
    pub source: (u32, u32, u32, u32),
    /// Kitty draws a source rectangle 1:1 unless told how many cells to fill.
    pub cells: Option<(u32, u32)>,
    pub z: i32,
}

/// An image touching anything in `covered` is hidden: Kitty can't cut a hole in an image.
pub fn place(p: &Placement, pane: Rect, cell: (u32, u32), covered: &[Rect]) -> Option<Placed> {
    let (cw, ch) = cell;
    if cw == 0 || ch == 0 || p.size.0 == 0 || p.size.1 == 0 {
        return None;
    }
    let (x0, sx, sw, ox, cols) = clip(p.col, p.offset.0, p.size.0, p.source.0, p.source.2, pane.w, cw)?;
    let (y0, sy, sh, oy, rows) = clip(p.row, p.offset.1, p.size.1, p.source.1, p.source.3, pane.h, ch)?;
    let screen = Rect::new(pane.x + x0, pane.y + y0, cols as u16, rows as u16);
    if covered.iter().any(|c| overlaps(*c, screen)) {
        return None;
    }
    let drawn = (cols * cw - ox, rows * ch - oy);
    let scaled = p.size.0 != p.source.2 || p.size.1 != p.source.3;
    let cells = scaled.then(|| (((drawn.0 + ox + cw / 2) / cw).max(1), ((drawn.1 + oy + ch / 2) / ch).max(1)));
    Some(Placed { x: screen.x, y: screen.y, offset: (ox, oy), source: (sx, sy, sw, sh), cells, z: p.z })
}

fn clip(
    cell0: i32,
    offset: u32,
    size: u32,
    src: u32,
    src_len: u32,
    cells: u16,
    cell: u32,
) -> Option<(u16, u32, u32, u32, u32)> {
    let start = cell0 as i64 * cell as i64 + offset as i64;
    let end = start + size as i64;
    let (a, b) = (start.max(0), end.min(cells as i64 * cell as i64));
    if b <= a {
        return None;
    }
    let scale = |px: i64| (px as u128 * src_len as u128 / size as u128) as u32;
    let src0 = src + scale(a - start);
    let len = scale(b - start) - scale(a - start);
    let first = (a / cell as i64) as u32;
    let off = (a - first as i64 * cell as i64) as u32;
    let covered = ((off as i64 + (b - a) + cell as i64 - 1) / cell as i64) as u32;
    Some((first as u16, src0, len.max(1), off, covered))
}

fn overlaps(a: Rect, b: Rect) -> bool {
    a.x < b.right() && b.x < a.right() && a.y < b.bottom() && b.y < a.bottom()
}

/// `pixels` is RGB, or RGBA with `alpha`.
pub fn transmit(out: &mut Vec<u8>, id: u32, width: u32, height: u32, alpha: bool, pixels: &[u8]) {
    let encoded = base64(pixels);
    let chunks: Vec<&[u8]> = if encoded.is_empty() { vec![&[][..]] } else { encoded.chunks(4096).collect() };
    for (i, chunk) in chunks.iter().enumerate() {
        let more = u8::from(i + 1 < chunks.len());
        if i == 0 {
            let f = if alpha { 32 } else { 24 };
            out.extend_from_slice(format!("\x1b_Ga=t,q=2,f={f},s={width},v={height},i={id},m={more};").as_bytes());
        } else {
            out.extend_from_slice(format!("\x1b_Gq=2,m={more};").as_bytes());
        }
        out.extend_from_slice(chunk);
        out.extend_from_slice(b"\x1b\\");
    }
}

/// Moves the cursor, so callers save and restore it around a batch.
pub fn put(out: &mut Vec<u8>, id: u32, placement: u32, p: &Placed) {
    let (x, y, w, h) = p.source;
    let mut cmd = format!(
        "\x1b[{};{}H\x1b_Ga=p,q=2,C=1,i={id},p={placement},x={x},y={y},w={w},h={h},X={},Y={},z={}",
        p.y + 1,
        p.x + 1,
        p.offset.0,
        p.offset.1,
        p.z
    );
    if let Some((c, r)) = p.cells {
        cmd.push_str(&format!(",c={c},r={r}"));
    }
    out.extend_from_slice(cmd.as_bytes());
    out.extend_from_slice(b"\x1b\\");
}

pub fn remove_placement(out: &mut Vec<u8>, id: u32, placement: u32) {
    out.extend_from_slice(format!("\x1b_Ga=d,q=2,d=i,i={id},p={placement}\x1b\\").as_bytes());
}

/// Frees the outer terminal's copy of the pixels too.
pub fn remove_image(out: &mut Vec<u8>, id: u32) {
    out.extend_from_slice(format!("\x1b_Ga=d,q=2,d=I,i={id}\x1b\\").as_bytes());
}

pub const REMOVE_ALL: &[u8] = b"\x1b_Ga=d,q=2,d=A\x1b\\";

fn base64(data: &[u8]) -> Vec<u8> {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let n =
            (chunk[0] as u32) << 16 | (*chunk.get(1).unwrap_or(&0) as u32) << 8 | *chunk.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            out.push(if i <= chunk.len() { ABC[(n >> (18 - 6 * i) & 63) as usize] } else { b'=' });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const CELL: (u32, u32) = (10, 20);

    fn at(col: i32, row: i32, w: u32, h: u32) -> Placement {
        Placement { image: 1, col, row, size: (w, h), source: (0, 0, w, h), ..Placement::default() }
    }

    #[test]
    fn inside_the_pane_it_moves_by_the_panes_offset() {
        let p = place(&at(2, 1, 30, 40), Rect::new(5, 3, 20, 10), CELL, &[]).unwrap();
        assert_eq!((p.x, p.y, p.source, p.cells), (7, 4, (0, 0, 30, 40), None));
    }

    #[test]
    fn clipped_at_the_top_left_it_draws_the_rest_of_the_image() {
        let p = place(&at(-1, -2, 30, 60), Rect::new(0, 0, 20, 10), CELL, &[]).unwrap();
        assert_eq!((p.x, p.y), (0, 0));
        assert_eq!(p.source, (10, 40, 20, 20));
    }

    #[test]
    fn clipped_at_the_bottom_right_and_gone_when_wholly_outside() {
        let p = place(&at(18, 9, 40, 40), Rect::new(0, 0, 20, 10), CELL, &[]).unwrap();
        assert_eq!(p.source, (0, 0, 20, 20));
        assert!(place(&at(25, 0, 10, 10), Rect::new(0, 0, 20, 10), CELL, &[]).is_none());
        assert!(place(&at(0, -5, 10, 40), Rect::new(0, 0, 20, 10), CELL, &[]).is_none());
    }

    #[test]
    fn a_scaled_image_scales_its_source_and_names_its_cells() {
        let mut s = at(-1, 0, 40, 40);
        s.source = (0, 0, 80, 80);
        let p = place(&s, Rect::new(0, 0, 20, 10), CELL, &[]).unwrap();
        assert_eq!(p.source, (20, 0, 60, 80));
        assert_eq!(p.cells, Some((3, 2)));
    }

    #[test]
    fn anything_drawn_over_it_hides_it() {
        assert!(place(&at(0, 0, 30, 40), Rect::new(0, 0, 20, 10), CELL, &[Rect::new(2, 1, 5, 5)]).is_none());
        assert!(place(&at(0, 0, 30, 40), Rect::new(0, 0, 20, 10), CELL, &[Rect::new(10, 5, 5, 5)]).is_some());
    }

    #[test]
    fn transmission_is_chunked_quiet_base64() {
        let mut out = Vec::new();
        transmit(&mut out, 7, 1, 1, true, &[1, 2, 3, 4]);
        assert_eq!(out, b"\x1b_Ga=t,q=2,f=32,s=1,v=1,i=7,m=0;AQIDBA==\x1b\\");
        let mut big = Vec::new();
        transmit(&mut big, 7, 64, 64, false, &vec![0u8; 64 * 64 * 3]);
        let s = String::from_utf8(big).unwrap();
        assert!(
            s.matches("\x1b_G").count() > 1
                && s.contains("m=1;")
                && s.ends_with("\x1b\\")
                && s.contains("\x1b_Gq=2,m=0;")
        );
    }

    #[test]
    fn base64_pads() {
        assert_eq!(base64(b"Ma"), b"TWE=");
        assert_eq!(base64(b"M"), b"TQ==");
        assert_eq!(base64(b"Man"), b"TWFu");
    }
}
