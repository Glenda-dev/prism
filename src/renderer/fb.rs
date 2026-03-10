use super::Renderer;
use super::font::FontRenderer;
use glenda::error::Error;
use glenda::protocol::terminal::TerminalDisplayMode;

pub struct FramebufferRenderer {
    pub width: usize,
    pub height: usize,
    pub pitch: usize,
    pub bpp: usize,
    pub fb_ptr: *mut u8,
    font: FontRenderer,
    mode: TerminalDisplayMode,
}

impl FramebufferRenderer {
    pub fn new(fb_ptr: *mut u8, width: usize, height: usize, pitch: usize, bpp: usize) -> Self {
        Self {
            fb_ptr,
            width,
            height,
            pitch,
            bpp,
            font: FontRenderer::new(),
            mode: TerminalDisplayMode::Text,
        }
    }
}

impl Renderer for FramebufferRenderer {
    fn clear(&mut self) -> Result<(), Error> {
        let bg_color = 0x00000000;
        for y in 0..self.height {
            for x in 0..self.width {
                self.draw_pixel_raw(x, y, bg_color);
            }
        }
        Ok(())
    }

    fn draw_char(
        &mut self,
        row: usize,
        col: usize,
        c: char,
        fg: u32,
        bg: u32,
    ) -> Result<(), Error> {
        let x = col * 8;
        let y = row * 16;
        let fb_ptr = self.fb_ptr;
        let pitch = self.pitch;
        let width_val = self.width;
        let height_val = self.height;
        let bpp = self.bpp;

        if let Some((glyph, char_width, char_height)) = self.font.get_char_bitmap(c) {
            let bytes_per_row = (char_width + 7) / 8;
            for r in 0..char_height {
                if y + r >= height_val {
                    break;
                }
                for c in 0..char_width {
                    if x + c >= width_val {
                        break;
                    }
                    let byte_offset = r * bytes_per_row + (c / 8);
                    let bit_offset = 7 - (c % 8);
                    let pixel_on = (glyph[byte_offset] & (1 << bit_offset)) != 0;
                    let color = if pixel_on { fg } else { bg };

                    let fb_offset = (y + r) * pitch + (x + c) * (bpp / 8);
                    unsafe {
                        let ptr = fb_ptr.add(fb_offset) as *mut u32;
                        ptr.write_volatile(color);
                    }
                }
            }
        }
        Ok(())
    }

    fn render_grid(
        &mut self,
        grid: &[u32],
        rows: usize,
        cols: usize,
        fg: u32,
        bg: u32,
    ) -> Result<(), Error> {
        // Simple ANSI parsing for FB
        let mut row = 0;
        let mut col = 0;
        let fg_color = fg;
        let bg_color = bg;

        let mut i = 0;
        while i < grid.len() && row < rows {
            let code = grid[i];
            i += 1;

            if let Some(ch) = core::char::from_u32(code) {
                match ch {
                    '\n' => {
                        row += 1;
                        col = 0;
                        continue;
                    }
                    '\r' => {
                        col = 0;
                        continue;
                    }
                    '\x1b' => {
                        // Very basic ANSI skip or parse for future
                        // For now we just skip the '[' and any numbers until 'm' or 'H' etc.
                        if i < grid.len() && grid[i] == b'[' as u32 {
                            i += 1;
                            while i < grid.len() {
                                let c = grid[i];
                                i += 1;
                                if (c >= b'a' as u32 && c <= b'z' as u32)
                                    || (c >= b'A' as u32 && c <= b'Z' as u32)
                                {
                                    break;
                                }
                            }
                        }
                        continue;
                    }
                    _ => {
                        if col < cols {
                            self.draw_char(row, col, ch, fg_color, bg_color)?;
                            col += 1;
                        }
                    }
                }
            }

            if col >= cols {
                col = 0;
                row += 1;
            }
        }
        Ok(())
    }

    fn set_mode(&mut self, mode: TerminalDisplayMode) -> Result<(), Error> {
        self.mode = mode;
        Ok(())
    }

    fn load_font(&mut self, data: &'static [u8]) -> Result<(), Error> {
        self.font.load_font(data).map_err(|_| Error::InvalidArgs)
    }

    fn draw_pixel(&mut self, x: usize, y: usize, color: u32) -> Result<(), Error> {
        self.draw_pixel_raw(x, y, color);
        Ok(())
    }

    fn draw_bitmap(
        &mut self,
        x: usize,
        y: usize,
        width: usize,
        height: usize,
        data: &[u32],
    ) -> Result<(), Error> {
        for dy in 0..height {
            for dx in 0..width {
                self.draw_pixel_raw(x + dx, y + dy, data[dy * width + dx]);
            }
        }
        Ok(())
    }

    fn flush(&mut self) -> Result<(), Error> {
        Ok(())
    }

    fn name(&self) -> &str {
        "FB"
    }

    fn set_buffer(&mut self, _paddr: usize, vaddr: *mut u8) -> Result<(), Error> {
        self.fb_ptr = vaddr;
        Ok(())
    }
}

impl FramebufferRenderer {
    pub fn draw_pixel_raw(&mut self, x: usize, y: usize, color: u32) {
        if x < self.width && y < self.height {
            let offset = y * self.pitch + x * (self.bpp / 8);
            unsafe {
                let ptr = self.fb_ptr.add(offset) as *mut u32;
                ptr.write_volatile(color);
            }
        }
    }
}

impl FramebufferRenderer {
    pub fn load_font(&mut self, data: &'static [u8]) -> Result<(), Error> {
        self.font.load_font(data)
    }

    pub fn draw_text(
        &mut self,
        row: usize,
        col: usize,
        s: &str,
        fg: u32,
        bg: u32,
    ) -> Result<(), Error> {
        let mut curr_col = col;
        for c in s.chars() {
            self.draw_char(row, curr_col, c, fg, bg)?;
            curr_col += 1;
        }
        Ok(())
    }
}
