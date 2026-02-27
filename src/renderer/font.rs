// use psf2_font::Font;

pub struct FontRenderer {
    // font: Option<Font<'static>>,
}

impl FontRenderer {
    pub const fn new() -> Self {
        Self { /* font: None */ }
    }

    pub fn load_font(&mut self, _data: &'static [u8]) -> Result<(), ()> {
        // let font = Font::new(data).map_err(|_| ())?;
        // self.font = Some(font);
        Ok(())
    }

    pub fn get_char_bitmap(&self, _c: char) -> Option<(&'static [u8], usize, usize)> {
        // If we have a dynamic font, use it.
        /*
        if let Some(font) = &self.font {
            // let glyph = font.get_glyph(c)?;
            // Some((glyph.data(), glyph.width(), glyph.height()))
            None
        } else {
            // Built-in fallback disabled for now due to psf2-font API mismatch
            None
        }
        */
        None
    }
}
