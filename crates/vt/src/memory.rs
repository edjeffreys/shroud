//! Scrollback limits and compression. History is kept in whole pages of about 400 KB, so every
//! limit is approximate.

use std::ffi::c_void;

use crate::ffi::*;
use crate::terminal::Terminal;
use crate::{Error, check};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Memory {
    /// Compressed pages count at their compressed size.
    pub resident: u64,
    pub images: u64,
    pub compressed_pages: u64,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Compression {
    Unsupported,
    Pending,
    Complete,
}

impl Terminal {
    /// `None` is unlimited.
    pub fn set_scrollback_bytes(&mut self, limit: Option<usize>) -> Result<(), Error> {
        let bytes = limit.unwrap_or(usize::MAX);
        self.set(GHOSTTY_TERMINAL_OPT_SCROLLBACK_MAX_BYTES, &bytes as *const usize as *const c_void)
    }

    pub fn memory(&self) -> Result<Memory, Error> {
        // SAFETY: the struct is plain data; zero is a valid value for every field.
        let mut m: GhosttyTerminalMemoryUsage = unsafe { std::mem::zeroed() };
        m.size = std::mem::size_of::<GhosttyTerminalMemoryUsage>();
        self.get(GHOSTTY_TERMINAL_DATA_MEMORY_USAGE, &mut m as *mut _ as *mut c_void)?;
        Ok(Memory {
            resident: m.primary_resident_bytes + m.alternate_resident_bytes,
            images: m.primary_image_bytes + m.alternate_image_bytes,
            compressed_pages: m.primary_compressed_pages + m.alternate_compressed_pages,
        })
    }

    /// Compression has nothing to do until this changes.
    pub fn compression_activity(&self) -> u64 {
        let mut token = 0u64;
        // SAFETY: raw is live and the out-pointer is a u64.
        let _ = unsafe { ghostty_terminal_compression_activity(self.raw(), &mut token) };
        token
    }

    /// Bounded work, safe to run between other events.
    pub fn compress_step(&mut self) -> Result<Compression, Error> {
        let mut r: GhosttyTerminalCompressionResult = GHOSTTY_TERMINAL_COMPRESSION_RESULT_UNSUPPORTED;
        // SAFETY: raw is live and the out-pointer matches the result type.
        check(unsafe { ghostty_terminal_compress(self.raw(), GHOSTTY_TERMINAL_COMPRESSION_MODE_INCREMENTAL, &mut r) })?;
        Ok(match r {
            GHOSTTY_TERMINAL_COMPRESSION_RESULT_PENDING => Compression::Pending,
            GHOSTTY_TERMINAL_COMPRESSION_RESULT_COMPLETE => Compression::Complete,
            _ => Compression::Unsupported,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flood(t: &mut Terminal, lines: usize) {
        let line = "a line of scrollback with some width to it 0123456789 abcdefghijklmnopqrstuvwxyz\r\n".repeat(1000);
        for _ in 0..lines / 1000 {
            t.write(line.as_bytes());
        }
    }

    #[test]
    fn the_byte_limit_bounds_history_and_none_lifts_it() {
        let mut capped = Terminal::new(80, 24).unwrap();
        capped.set_scrollback_bytes(Some(2 << 20)).unwrap();
        flood(&mut capped, 100_000);
        let mut open = Terminal::new(80, 24).unwrap();
        open.set_scrollback_bytes(None).unwrap();
        flood(&mut open, 100_000);
        let (c, o) = (capped.memory().unwrap().resident, open.memory().unwrap().resident);
        assert!(c < 4 << 20, "capped at {c}");
        assert!(o > 2 * c, "unlimited at {o}, capped at {c}");
        assert!(open.viewport().total > 10 * capped.viewport().total / 2);
    }

    #[test]
    fn compression_shrinks_repetitive_history_and_then_rests() {
        let mut t = Terminal::new(80, 24).unwrap();
        t.set_scrollback_bytes(None).unwrap();
        flood(&mut t, 50_000);
        let before = t.memory().unwrap();
        let token = t.compression_activity();
        let mut steps = 0;
        loop {
            match t.compress_step().unwrap() {
                Compression::Pending if steps < 10_000 => steps += 1,
                Compression::Unsupported => return,
                _ => break,
            }
        }
        let after = t.memory().unwrap();
        assert!(after.compressed_pages > 0 && after.resident < before.resident, "{before:?} → {after:?}");
        assert_eq!(t.compression_activity(), token, "compressing isn't activity");
    }
}
