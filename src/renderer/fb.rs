use super::font::FontRenderer;

pub struct FramebufferRenderer {
    pub width: usize,
    pub height: usize,
    pub pitch: usize,
    pub bpp: usize,
    pub fb_ptr: *mut u8,
    font: FontRenderer,
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
        }
    }

    pub fn draw_pixel(&mut self, x: usize, y: usize, color: u32) {
        if x < self.width && y < self.height {
            let offset = y * self.pitch + x * (self.bpp / 8);
            unsafe {
                let ptr = self.fb_ptr.add(offset) as *mut u32;
                ptr.write_volatile(color);
            }
        }
    }

    pub fn draw_char(&mut self, x: usize, y: usize, c: char, fg: u32, bg: u32) {
        let fb_ptr = self.fb_ptr;
        let pitch = self.pitch;
        let width_val = self.width;
        let height_val = self.height;
        let bpp = self.bpp;
        
        if let Some((glyph, char_width, char_height)) = self.font.get_char_bitmap(c) {
            let bytes_per_row = (char_width + 7) / 8;
            for row in 0..char_height {
                if y + row >= height_val { break; }
                for col in 0..char_width {
                    if x + col >= width_val { break; }
                    let byte_offset = row * bytes_per_row + (col / 8);
                    let bit_offset = 7 - (col % 8);
                    let pixel_on = (glyph[byte_offset] & (1 << bit_offset)) != 0;
                    let color = if pixel_on { fg } else { bg };
                    
                    let fb_offset = (y + row) * pitch + (x + col) * (bpp / 8);
                    unsafe {
                        let ptr = fb_ptr.add(fb_offset) as *mut u32;
                        ptr.write_volatile(color);
                    }
                }
            }
        }
    }

    pub fn load_font(&mut self, data: &'static [u8]) -> Result<(), ()> {
        self.font.load_font(data)
    }

    pub fn draw_text(&mut self, x: usize, y: usize, s: &str, fg: u32, bg: u32) {
        let mut curr_x = x;
        // Collect all chars first to avoid borrowing self.font while using self.draw_char
        // Or better, just loop and get bitmaps one by one.
        for c in s.chars() {
            let res = self.font.get_char_bitmap(c);
            if let Some((_, char_width, _)) = res {
                self.draw_char(curr_x, y, c, fg, bg);
                curr_x += char_width;
            }
        }
    }
}
