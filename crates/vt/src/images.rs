//! Kitty graphics read out for re-sending. libghostty-vt's pointers die at the next write, so
//! everything is copied out at once.

use std::ffi::c_void;

use shroud_core::render::images::Placement;

use crate::ffi::*;
use crate::snapshot::Snapshot;
use crate::terminal::Terminal;
use crate::{Error, check};

/// Grey is widened to RGB: the outer terminal takes only RGB and RGBA.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub alpha: bool,
    pub pixels: Vec<u8>,
}

impl Snapshot {
    /// `None` when unchanged since the last call.
    pub fn placements(&mut self) -> Result<Option<Vec<Placement>>, Error> {
        let mut generation = 0u64;
        // SAFETY: the key's output type is u64.
        unsafe { self.get(GHOSTTY_RENDER_STATE_DATA_IMAGE_PLACEMENTS_GENERATION, &mut generation)? };
        if generation == self.images_seen {
            return Ok(None);
        }
        self.images_seen = generation;
        let mut out = Vec::new();
        let mut it = self.images;
        // SAFETY: the iterator is live and refilled from this update before it's read; each
        // read's out-pointer matches the type documented for its key.
        unsafe {
            self.get(GHOSTTY_RENDER_STATE_DATA_IMAGE_ITERATOR, &mut it)?;
            while ghostty_render_state_image_iterator_next(self.images) {
                let mut protocol: GhosttyImageProtocol = 0;
                self.image_get(GHOSTTY_RENDER_STATE_IMAGE_DATA_PROTOCOL, &mut protocol)?;
                if protocol != GHOSTTY_IMAGE_PROTOCOL_KITTY {
                    continue;
                }
                let mut p = Placement::default();
                let (mut ox, mut oy, mut w, mut h) = (0u32, 0u32, 0u32, 0u32);
                let (mut sx, mut sy, mut sw, mut sh) = (0u32, 0u32, 0u32, 0u32);
                self.image_get(GHOSTTY_RENDER_STATE_IMAGE_DATA_IMAGE_ID, &mut p.image)?;
                self.image_get(GHOSTTY_RENDER_STATE_IMAGE_DATA_IMAGE_GENERATION, &mut p.generation)?;
                self.image_get(GHOSTTY_RENDER_STATE_IMAGE_DATA_Z, &mut p.z)?;
                self.image_get(GHOSTTY_RENDER_STATE_IMAGE_DATA_VIEWPORT_COL, &mut p.col)?;
                self.image_get(GHOSTTY_RENDER_STATE_IMAGE_DATA_VIEWPORT_ROW, &mut p.row)?;
                self.image_get(GHOSTTY_RENDER_STATE_IMAGE_DATA_CELL_OFFSET_X, &mut ox)?;
                self.image_get(GHOSTTY_RENDER_STATE_IMAGE_DATA_CELL_OFFSET_Y, &mut oy)?;
                self.image_get(GHOSTTY_RENDER_STATE_IMAGE_DATA_PIXEL_WIDTH, &mut w)?;
                self.image_get(GHOSTTY_RENDER_STATE_IMAGE_DATA_PIXEL_HEIGHT, &mut h)?;
                self.image_get(GHOSTTY_RENDER_STATE_IMAGE_DATA_SOURCE_X, &mut sx)?;
                self.image_get(GHOSTTY_RENDER_STATE_IMAGE_DATA_SOURCE_Y, &mut sy)?;
                self.image_get(GHOSTTY_RENDER_STATE_IMAGE_DATA_SOURCE_WIDTH, &mut sw)?;
                self.image_get(GHOSTTY_RENDER_STATE_IMAGE_DATA_SOURCE_HEIGHT, &mut sh)?;
                (p.offset, p.size, p.source) = ((ox, oy), (w, h), (sx, sy, sw, sh));
                out.push(p);
            }
        }
        // Layers are z ranges, so sorting by z alone keeps them in order.
        out.sort_by_key(|p| p.z);
        Ok(Some(out))
    }

    unsafe fn image_get<T>(&self, key: GhosttyRenderStateImageData, out: &mut T) -> Result<(), Error> {
        check(unsafe { ghostty_render_state_image_get(self.images, key, out as *mut T as *mut c_void) })
    }
}

impl Terminal {
    pub fn image(&self, id: u32) -> Option<Image> {
        let mut storage: GhosttyKittyGraphics = std::ptr::null_mut();
        self.get(GHOSTTY_TERMINAL_DATA_KITTY_GRAPHICS, &mut storage as *mut _ as *mut c_void).ok()?;
        // SAFETY: borrowed from the terminal, which isn't written meanwhile; pixels are copied.
        unsafe {
            let img = ghostty_kitty_graphics_image(storage, id);
            if img.is_null() {
                return None;
            }
            let (mut width, mut height) = (0u32, 0u32);
            let mut format: GhosttyKittyImageFormat = 0;
            let mut px = GhosttyString { ptr: std::ptr::null(), len: 0 };
            let get = |k, out: *mut c_void| ghostty_kitty_graphics_image_get(img, k, out) == GHOSTTY_SUCCESS;
            if !(get(GHOSTTY_KITTY_GRAPHICS_IMAGE_DATA_WIDTH, &mut width as *mut _ as *mut c_void)
                && get(GHOSTTY_KITTY_GRAPHICS_IMAGE_DATA_HEIGHT, &mut height as *mut _ as *mut c_void)
                && get(GHOSTTY_KITTY_GRAPHICS_IMAGE_DATA_FORMAT, &mut format as *mut _ as *mut c_void)
                && get(GHOSTTY_KITTY_GRAPHICS_IMAGE_DATA_PIXELS, &mut px as *mut _ as *mut c_void))
                || px.ptr.is_null()
            {
                return None;
            }
            let raw = std::slice::from_raw_parts(px.ptr, px.len);
            widen(width, height, format, raw)
        }
    }

    /// Local programs send big images by file or shared memory; they run as the user anyway.
    pub(crate) fn allow_image_media(&self) {
        let on = true;
        let _ = self.set(GHOSTTY_TERMINAL_OPT_KITTY_IMAGE_MEDIUM_FILE, &on as *const bool as *const c_void);
        let _ = self.set(GHOSTTY_TERMINAL_OPT_KITTY_IMAGE_MEDIUM_SHARED_MEM, &on as *const bool as *const c_void);
        let tmp = std::env::temp_dir();
        let tmp = tmp.to_string_lossy();
        let dir = GhosttyString { ptr: tmp.as_ptr(), len: tmp.len() };
        let _ =
            self.set(GHOSTTY_TERMINAL_OPT_KITTY_IMAGE_MEDIUM_TEMP_FILE, &dir as *const GhosttyString as *const c_void);
    }
}

fn widen(width: u32, height: u32, format: GhosttyKittyImageFormat, raw: &[u8]) -> Option<Image> {
    let (alpha, pixels) = match format {
        GHOSTTY_KITTY_IMAGE_FORMAT_RGB => (false, raw.to_vec()),
        GHOSTTY_KITTY_IMAGE_FORMAT_RGBA => (true, raw.to_vec()),
        GHOSTTY_KITTY_IMAGE_FORMAT_GRAY => (false, raw.iter().flat_map(|&g| [g, g, g]).collect()),
        GHOSTTY_KITTY_IMAGE_FORMAT_GRAY_ALPHA => {
            (true, raw.as_chunks::<2>().0.iter().flat_map(|c| [c[0], c[0], c[0], c[1]]).collect())
        }
        _ => return None,
    };
    let want = width as usize * height as usize * if alpha { 4 } else { 3 };
    (pixels.len() == want).then_some(Image { width, height, alpha, pixels })
}

#[cfg(test)]
mod tests {
    use super::*;
    use shroud_core::render::{Grid, Links};

    #[test]
    fn an_image_sent_by_a_program_is_listed_with_its_pixels() {
        let mut t = Terminal::new(20, 5).unwrap();
        t.resize(20, 5, 10, 20).unwrap();
        t.write(b"\x1b[2;4H\x1b_Ga=T,f=24,s=2,v=1,i=9,q=2;/wAAAP8A\x1b\\");
        let mut s = Snapshot::new().unwrap();
        let (mut g, mut l) = (Grid::new(20, 5), Links::default());
        s.update(&t, &mut g, &mut l).unwrap();
        let list = s.placements().unwrap().expect("first read lists them");
        assert_eq!(list.len(), 1, "{list:?}");
        let p = list[0];
        assert_eq!((p.image, p.col, p.row, p.size, p.source), (9, 3, 1, (2, 1), (0, 0, 2, 1)));
        assert_eq!(s.placements().unwrap(), None, "unchanged since");
        let img = t.image(9).unwrap();
        assert_eq!((img.width, img.height, img.alpha, img.pixels), (2, 1, false, vec![255, 0, 0, 0, 255, 0]));
    }

    #[test]
    fn grey_is_widened() {
        let img = widen(2, 1, GHOSTTY_KITTY_IMAGE_FORMAT_GRAY_ALPHA, &[10, 255, 20, 0]).unwrap();
        assert_eq!(img.pixels, vec![10, 10, 10, 255, 20, 20, 20, 0]);
        assert!(widen(2, 2, GHOSTTY_KITTY_IMAGE_FORMAT_RGB, &[0; 3]).is_none(), "short data");
    }
}
