//! Colour references, never resolved to RGB, so the outer terminal's theme applies.
//! No concrete chrome colours here; those belong to `theme`.

#[derive(Clone, Copy, Eq, Debug, Default)]
pub enum Color {
    #[default]
    Default,
    Palette(u8),
    Rgb(u8, u8, u8),
}

impl Color {
    #[inline]
    const fn key(self) -> u32 {
        match self {
            Self::Default => 0,
            Self::Palette(n) => 1 << 24 | n as u32,
            Self::Rgb(r, g, b) => 2 << 24 | (r as u32) << 16 | (g as u32) << 8 | b as u32,
        }
    }
}

impl std::hash::Hash for Color {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.key().hash(state);
    }
}

/// Branch-free, because diff compares every cell of every frame.
impl PartialEq for Color {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.key() == other.key()
    }
}
