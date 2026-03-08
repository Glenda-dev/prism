use crate::prism::seat::Seat;
use crate::prism::vt::VirtualTerminal;
use crate::renderer::FramebufferRenderer;
use crate::prism::device::DeviceResource;
use alloc::collections::BTreeMap;
use alloc::string::String;

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

    pub fn clear_vt(&mut self, vt: &mut VirtualTerminal, input_devices: &mut BTreeMap<String, DeviceResource>, output_devices: &mut BTreeMap<String, DeviceResource>) {
        vt.grid.fill(b' ' as u32);
        vt.cursor = (0, 0);

        // Clear FB if we have one assigned
        if let Some(renderer) = self.renderer.as_mut() {
             // Fill screen with background color
             let bg_color = 0x00000000;
             for y in 0..renderer.height {
                 for x in 0..renderer.width {
                     renderer.draw_pixel(x, y, bg_color);
                 }
             }
        }

        // Send clear sequence to UARTs (ANSI clear)
        let clear_seq = "\x1b[2J\x1b[H";
        self.output_to_devices(vt, clear_seq, input_devices, output_devices);
    }

    /// Output string to all output devices associated with VT's seats
    pub fn output_to_devices(&mut self, vt: &VirtualTerminal, s: &str, input_devices: &mut BTreeMap<String, DeviceResource>, output_devices: &mut BTreeMap<String, DeviceResource>) {
        // Simple logic: if it's VT0 (System VT), output to all devices on Seat 0
        // In a more complex system, we'd lookup which seats this VT is bound to.
        if vt.id == 0 {
             // For each device in input/output maps, if they are UARTs, write to them
             for dev in input_devices.values_mut() {
                 if dev.is_output() {
                     let _ = dev.write_str(s);
                 }
             }
             for dev in output_devices.values_mut() {
                 if dev.is_output() {
                     let _ = dev.write_str(s);
                 }
             }
        }
    }

    /// Mix/Compose outputs for current display.
    pub fn render_vt(&mut self, vt: &VirtualTerminal, _input_devices: &mut BTreeMap<String, DeviceResource>, _output_devices: &mut BTreeMap<String, DeviceResource>) {
        // FB Rendering
        if let Some(renderer) = self.renderer.as_mut() {
            // Background fill
            let bg_color = 0x00000000;
            let fg_color = 0x00FFFFFF;

            for r in 0..vt.winsize.rows as usize {
                for c in 0..vt.winsize.cols as usize {
                    let char_idx = r * vt.winsize.cols as usize + c;
                    if let Some(ch_u32) = vt.grid.get(char_idx) {
                        if let Some(ch) = core::char::from_u32(*ch_u32) {
                            renderer.draw_char(c * 8, r * 16, ch, fg_color, bg_color);
                        }
                    }
                }
            }
        }

        // For UARTs, the render_vt call usually follows a write_str to the VT.
        // The write_str itself should have triggered output_to_devices.
        // So we don't necessarily need to re-scan the grid for UARTs here unless it's a full refresh.
    }
}

