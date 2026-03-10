use glenda::error::Error;
use psf2_font::Psf2Font;

/// Simple font renderer that wraps a PSF2 font instance.  The font data is
/// provided as a `'static` slice (usually coming from the resource loader or
/// an embedded fallback), but once parsed we keep an owned `Psf2Font` which
/// allocates internally via `alloc`.  The glyph bitmaps are therefore owned by
/// the renderer and have the same lifetime as `&self`.
pub struct FontRenderer {
    font: Option<Psf2Font>,
}

impl FontRenderer {
    pub const fn new() -> Self {
        Self { font: None }
    }

    /// Parse a PSF2 font from the supplied data.  The caller is responsible for
    /// keeping `data` alive for as long as the renderer might need to reload or
    /// re‑parse it (typically it is `'static`).
    pub fn load_font(&mut self, data: &'static [u8]) -> Result<(), Error> {
        let parsed = Psf2Font::parse(data).map_err(|_| Error::InvalidArgs)?;
        self.font = Some(parsed);
        Ok(())
    }

    /// Return the bitmap for a character along with its width/height.  The
    /// returned slice borrows from the internally‑owned `Psf2Font` so its
    /// lifetime is tied to `&self` (not `'static`).
    pub fn get_char_bitmap(&self, c: char) -> Option<(&[u8], usize, usize)> {
        if let Some(font) = &self.font {
            if let Some(bitmap) = font.get_glyph(c) {
                let (w, h) = font.dimensions();
                return Some((bitmap, w as usize, h as usize));
            }
        }
        None
    }
}
