pub mod fb;
pub mod font;

use glenda::error::Error;
use glenda::protocol::terminal::TerminalDisplayMode;

pub trait Renderer {
    /// Clear the display surface.
    fn clear(&mut self) -> Result<(), Error>;

    /// Draw a single character at grid coordinates.
    fn draw_char(&mut self, row: usize, col: usize, c: char, fg: u32, bg: u32)
    -> Result<(), Error>;

    /// Render a full text grid.
    fn render_grid(
        &mut self,
        grid: &[u32],
        rows: usize,
        cols: usize,
        fg: u32,
        bg: u32,
    ) -> Result<(), Error>;

    /// Set terminal display mode (Text vs Graphic).
    fn set_mode(&mut self, mode: TerminalDisplayMode) -> Result<(), Error>;

    /// Load font data for rendering characters.
    fn load_font(&mut self, data: &'static [u8]) -> Result<(), Error>;

    /// Graphic mode specific methods
    fn draw_pixel(&mut self, _x: usize, _y: usize, _color: u32) -> Result<(), Error> {
        Ok(())
    }

    fn draw_bitmap(
        &mut self,
        _x: usize,
        _y: usize,
        _width: usize,
        _height: usize,
        _data: &[u32],
    ) -> Result<(), Error> {
        Ok(())
    }

    /// Push buffered changes to the actual hardware.
    fn flush(&mut self) -> Result<(), Error>;

    /// Get the type name of the renderer.
    fn name(&self) -> &str;

    /// Set the current buffer address (for zero-copy switch)
    fn set_buffer(&mut self, _paddr: usize, _vaddr: *mut u8) -> Result<(), Error> {
        Ok(())
    }
}
