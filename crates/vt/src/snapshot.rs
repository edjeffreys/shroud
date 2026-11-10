//! Copies style colours as the program set them, never libghostty-vt's resolved RGB,
//! so pane content keeps the user's theme.

use std::ffi::c_void;
use std::ptr;

use shroud_core::render::{Cell, Color, Cursor, CursorShape, Grid, Links, NO_LINK, Style, Underline, Width};

use crate::ffi::*;
use crate::terminal::Terminal;
use crate::{Error, check};

pub struct Snapshot {
    pub(crate) state: GhosttyRenderState,
    rows: GhosttyRenderStateRowIterator,
    cells: GhosttyRenderStateRowCells,
    pub(crate) images: GhosttyRenderStateImageIterator,
    pub(crate) images_seen: u64,
}

impl std::fmt::Debug for Snapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Snapshot").finish_non_exhaustive()
    }
}

impl Snapshot {
    pub fn new() -> Result<Snapshot, Error> {
        let mut s = Snapshot {
            state: ptr::null_mut(),
            rows: ptr::null_mut(),
            cells: ptr::null_mut(),
            images: ptr::null_mut(),
            images_seen: 0,
        };
        // SAFETY: out-pointers are valid; Drop frees whatever succeeded.
        unsafe {
            check(ghostty_render_state_new(ptr::null(), &mut s.state))?;
            check(ghostty_render_state_row_iterator_new(ptr::null(), &mut s.rows))?;
            check(ghostty_render_state_row_cells_new(ptr::null(), &mut s.cells))?;
            check(ghostty_render_state_image_iterator_new(ptr::null(), &mut s.images))?;
        }
        Ok(s)
    }

    pub fn update(&mut self, term: &Terminal, grid: &mut Grid, links: &mut Links) -> Result<bool, Error> {
        // SAFETY: all handles are live and owned by self; out-pointers match each key's type.
        unsafe {
            check(ghostty_render_state_update(self.state, term.raw()))?;
            let mut dirty: GhosttyRenderStateDirty = GHOSTTY_RENDER_STATE_DIRTY_FALSE;
            self.get(GHOSTTY_RENDER_STATE_DATA_DIRTY, &mut dirty)?;
            let (mut cols, mut rows) = (0u16, 0u16);
            self.get(GHOSTTY_RENDER_STATE_DATA_COLS, &mut cols)?;
            self.get(GHOSTTY_RENDER_STATE_DATA_ROWS, &mut rows)?;

            let resized = (grid.cols(), grid.rows()) != (cols, rows);
            grid.resize(cols, rows);
            let full = resized || dirty == GHOSTTY_RENDER_STATE_DIRTY_FULL;
            if !full && dirty == GHOSTTY_RENDER_STATE_DIRTY_FALSE {
                return Ok(false);
            }

            let mut rows_handle = self.rows;
            self.get(GHOSTTY_RENDER_STATE_DATA_ROW_ITERATOR, &mut rows_handle)?;
            let mut y: u16 = 0;
            loop {
                let more = if full {
                    ghostty_render_state_row_iterator_next(self.rows)
                } else {
                    ghostty_render_state_row_iterator_next_dirty(self.rows, &mut y)
                };
                if !more || y >= rows {
                    break;
                }
                self.copy_row(term, grid, y, links)?;
                if full {
                    y += 1;
                }
            }
            check(ghostty_render_state_clean(self.state))?;
        }
        Ok(true)
    }

    pub fn cursor(&mut self) -> Result<Cursor, Error> {
        let mut c: GhosttyRenderStateCursor = unsafe { std::mem::zeroed() };
        c.size = std::mem::size_of::<GhosttyRenderStateCursor>();
        // SAFETY: sized struct initialised as the API requires.
        unsafe { self.get(GHOSTTY_RENDER_STATE_DATA_CURSOR, &mut c)? };
        let shape = match c.visual_style {
            GHOSTTY_CURSOR_STYLE_BAR => CursorShape::Bar,
            GHOSTTY_CURSOR_STYLE_UNDERLINE => CursorShape::Underline,
            GHOSTTY_CURSOR_STYLE_BLOCK | GHOSTTY_CURSOR_STYLE_BLOCK_HOLLOW => CursorShape::Block,
            _ => CursorShape::Default,
        };
        Ok(Cursor {
            x: c.viewport_x,
            y: c.viewport_y,
            visible: c.visible && c.viewport_has_value,
            shape,
            blink: c.blinking,
        })
    }

    unsafe fn copy_row(&mut self, term: &Terminal, grid: &mut Grid, y: u16, links: &mut Links) -> Result<(), Error> {
        unsafe {
            check(ghostty_render_state_row_get(
                self.rows,
                GHOSTTY_RENDER_STATE_ROW_DATA_CELLS,
                &mut self.cells as *mut _ as *mut c_void,
            ))?;
            let row = grid.row_mut(y);
            let mut x = 0usize;
            while ghostty_render_state_row_cells_next(self.cells) && x < row.len() {
                let (mut cell, linked) = self.cell()?;
                if linked {
                    cell.link = term.hyperlink_at(x as u16, y).map_or(NO_LINK, |uri| links.intern(&uri));
                }
                row[x] = cell;
                x += 1;
            }
            row[x..].fill(Cell::BLANK);
        }
        Ok(())
    }

    unsafe fn cell(&mut self) -> Result<(Cell, bool), Error> {
        unsafe {
            let mut raw: GhosttyCell = 0;
            self.cell_get(GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_RAW, &mut raw)?;
            let mut wide: GhosttyCellWide = GHOSTTY_CELL_WIDE_NARROW;
            check(ghostty_cell_get(raw, GHOSTTY_CELL_DATA_WIDE, &mut wide as *mut _ as *mut c_void))?;
            let width = match wide {
                GHOSTTY_CELL_WIDE_WIDE => Width::Wide,
                GHOSTTY_CELL_WIDE_SPACER_TAIL => Width::Spacer,
                _ => Width::Narrow,
            };
            let mut style: GhosttyStyle = std::mem::zeroed();
            self.cell_get(GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_STYLE, &mut style)?;
            let mut s = convert_style(&style);

            // Erased-with-background cells carry their colour on the cell, not the style.
            let mut tag: GhosttyCellContentTag = 0;
            check(ghostty_cell_get(raw, GHOSTTY_CELL_DATA_CONTENT_TAG, &mut tag as *mut _ as *mut c_void))?;
            if tag == GHOSTTY_CELL_CONTENT_BG_COLOR_PALETTE {
                let mut i: GhosttyColorPaletteIndex = 0;
                check(ghostty_cell_get(raw, GHOSTTY_CELL_DATA_COLOR_PALETTE, &mut i as *mut _ as *mut c_void))?;
                s.bg = Color::Palette(i);
            } else if tag == GHOSTTY_CELL_CONTENT_BG_COLOR_RGB {
                let mut c = GhosttyColorRgb { r: 0, g: 0, b: 0 };
                check(ghostty_cell_get(raw, GHOSTTY_CELL_DATA_COLOR_RGB, &mut c as *mut _ as *mut c_void))?;
                s.bg = Color::Rgb(c.r, c.g, c.b);
            }

            let mut bytes = [0u8; 64];
            let mut buf = GhosttyBuffer { ptr: bytes.as_mut_ptr(), cap: bytes.len(), len: 0 };
            let r = ghostty_render_state_row_cells_get(
                self.cells,
                GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_GRAPHEMES_UTF8,
                &mut buf as *mut _ as *mut c_void,
            );
            let text = match r {
                GHOSTTY_SUCCESS => std::str::from_utf8(&bytes[..buf.len.min(bytes.len())]).unwrap_or("\u{FFFD}"),
                _ => "\u{FFFD}",
            };
            let mut linked = false;
            check(ghostty_cell_get(raw, GHOSTTY_CELL_DATA_HAS_HYPERLINK, &mut linked as *mut _ as *mut c_void))?;
            Ok((Cell::new(text, width, s), linked))
        }
    }

    pub(crate) unsafe fn get<T>(&self, key: GhosttyRenderStateData, out: &mut T) -> Result<(), Error> {
        check(unsafe { ghostty_render_state_get(self.state, key, out as *mut T as *mut c_void) })
    }

    unsafe fn cell_get<T>(&self, key: GhosttyRenderStateRowCellsData, out: &mut T) -> Result<(), Error> {
        check(unsafe { ghostty_render_state_row_cells_get(self.cells, key, out as *mut T as *mut c_void) })
    }
}

impl Drop for Snapshot {
    fn drop(&mut self) {
        // SAFETY: each handle is freed once; the free functions accept null.
        unsafe {
            ghostty_render_state_row_cells_free(self.cells);
            ghostty_render_state_image_iterator_free(self.images);
            ghostty_render_state_row_iterator_free(self.rows);
            ghostty_render_state_free(self.state);
        }
    }
}

fn convert_color(c: &GhosttyStyleColor) -> Color {
    // SAFETY: the tag says which union field is valid.
    unsafe {
        match c.tag as u32 {
            GHOSTTY_STYLE_COLOR_PALETTE => Color::Palette(c.value.palette),
            GHOSTTY_STYLE_COLOR_RGB => Color::Rgb(c.value.rgb.r, c.value.rgb.g, c.value.rgb.b),
            _ => Color::Default,
        }
    }
}

fn convert_style(s: &GhosttyStyle) -> Style {
    let f = s.flags;
    let bit = |n: u16| f & (1 << n) != 0;
    Style {
        fg: convert_color(&s.fg_color),
        bg: convert_color(&s.bg_color),
        underline_color: convert_color(&s.underline_color),
        bold: bit(0),
        italic: bit(1),
        faint: bit(2),
        blink: bit(3),
        inverse: bit(4),
        invisible: bit(5),
        strikethrough: bit(6),
        overline: bit(7),
        underline: match (f >> 8) & 0x7 {
            1 => Underline::Single,
            2 => Underline::Double,
            3 => Underline::Curly,
            4 => Underline::Dotted,
            5 => Underline::Dashed,
            _ => Underline::None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_colours_survive_and_wide_cells_are_marked() {
        let mut t = Terminal::new(10, 2).unwrap();
        t.write("\x1b[31mr\x1b[0m世\x1b[38;2;1;2;3mx".as_bytes());
        let mut snap = Snapshot::new().unwrap();
        let mut links = Links::default();
        let mut grid = Grid::new(1, 1);
        assert!(snap.update(&t, &mut grid, &mut links).unwrap());
        assert_eq!((grid.cols(), grid.rows()), (10, 2));
        assert_eq!(grid.get(0, 0).style.fg, Color::Palette(1));
        assert_eq!(grid.get(1, 0).text(), "世");
        assert_eq!(grid.get(1, 0).width, Width::Wide);
        assert_eq!(grid.get(2, 0).width, Width::Spacer);
        assert_eq!(grid.get(3, 0).style.fg, Color::Rgb(1, 2, 3));
        assert_eq!(snap.cursor().unwrap().x, 4);
        assert!(!snap.update(&t, &mut grid, &mut links).unwrap());
    }

    #[test]
    fn hyperlinks_become_link_ids() {
        let mut t = Terminal::new(10, 1).unwrap();
        t.write(b"\x1b]8;;https://a.test\x1b\\ab\x1b]8;;\x1b\\c");
        let mut snap = Snapshot::new().unwrap();
        let mut links = Links::default();
        let mut grid = Grid::new(10, 1);
        snap.update(&t, &mut grid, &mut links).unwrap();
        let id = grid.get(0, 0).link;
        assert_eq!(links.uri(id), Some("https://a.test"));
        assert_eq!(grid.get(1, 0).link, id);
        assert_eq!(grid.get(2, 0).link, NO_LINK);
    }

    #[test]
    fn grapheme_clustering_default_makes_one_wide_cell() {
        let mut t = Terminal::new(10, 1).unwrap();
        t.set_mode_default(2027, false, true);
        t.write("👍🏽|\x1bc👍🏽|".as_bytes());
        let mut snap = Snapshot::new().unwrap();
        let mut grid = Grid::new(10, 1);
        snap.update(&t, &mut grid, &mut Links::default()).unwrap();
        assert_eq!(grid.get(0, 0).text(), "👍🏽", "survives RIS");
        assert_eq!(grid.get(2, 0).text(), "|");
    }

    #[test]
    fn only_dirty_rows_are_rewritten() {
        let mut t = Terminal::new(4, 3).unwrap();
        let mut snap = Snapshot::new().unwrap();
        let mut links = Links::default();
        let mut grid = Grid::new(4, 3);
        snap.update(&t, &mut grid, &mut links).unwrap();
        grid.set(0, 2, Cell::new("Z", Width::Narrow, Style::PLAIN));
        t.write(b"a");
        snap.update(&t, &mut grid, &mut links).unwrap();
        assert_eq!(grid.get(0, 0).text(), "a");
        assert_eq!(grid.get(0, 2).text(), "Z", "an untouched row must not be recopied");
    }
}
