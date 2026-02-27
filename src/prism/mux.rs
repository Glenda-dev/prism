use crate::prism::seat::Seat;
use crate::prism::vt::VirtualTerminal;
use crate::renderer::FramebufferRenderer;

/// Multiplexer for terminal input routing and output mapping.
pub struct Muxer {
    pub renderer: Option<FramebufferRenderer>,
}

impl Muxer {
    pub fn new() -> Self {
        Self { renderer: None }
    }

    pub fn set_renderer(&mut self, renderer: FramebufferRenderer) {
        self.renderer = Some(renderer);
    }

    pub fn load_font(&mut self, data: &'static [u8]) -> Result<(), ()> {
        if let Some(renderer) = self.renderer.as_mut() {
            renderer.load_font(data)?;
        }
        Ok(())
    }
    pub fn handle_input(&mut self, _seat: &Seat, _device: &str, _data: &[u8]) {
    }

    /// Mix/Compose outputs for current display.
    pub fn render_vt(&mut self, vt: &VirtualTerminal) {
        if let Some(renderer) = self.renderer.as_mut() {
            // Background fill
            let bg_color = 0x00000000;
            let fg_color = 0x00FFFFFF;

            for r in 0..vt.winsize.rows as usize {
                for c in 0..vt.winsize.cols as usize {
                    let char_idx = r * vt.winsize.cols as usize + c;
                    if let Some(ch_u32) = vt.grid.get(char_idx) {
                        if let Some(ch) = core::char::from_u32(*ch_u32) {
                            // Unifont is 16px high, assume each char is 8 or 16px wide
                            // For simplicity, let's assume 16px font height.
                            renderer.draw_char(c * 8, r * 16, ch, fg_color, bg_color);
                        }
                    }
                }
            }
        }
    }
}

