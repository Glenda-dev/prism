use crate::prism::vt::VirtualTerminal;
use crate::renderer::Renderer;
use alloc::string::String;
use alloc::vec::Vec;
use glenda::error::Error;
use glenda::ipc::Badge;
use glenda::protocol::terminal::SeatDesc;

/// Represents a Seat (Input/Output grouping).
pub struct Seat<'a> {
    pub id: usize,
    pub name: String,
    pub active_vt: Option<usize>,
    pub input_devices: Vec<String>,
    pub output_devices: Vec<String>,
    pub renderers: Vec<alloc::boxed::Box<dyn Renderer + 'a>>,
    pub exclusive_owner: Option<Badge>, // Only this badge can switch VTs or input
}

impl<'a> Seat<'a> {
    pub fn new(id: usize, name: &str) -> Self {
        Self {
            id,
            name: String::from(name),
            active_vt: None,
            input_devices: Vec::new(),
            output_devices: Vec::new(),
            renderers: Vec::new(),
            exclusive_owner: None,
        }
    }

    pub fn render(&mut self, vt: &VirtualTerminal) -> Result<(), Error> {
        let (rows, cols) = (vt.winsize.rows as usize, vt.winsize.cols as usize);
        let fg = 0x00FFFFFF;
        let bg = 0x00000000;

        for r in self.renderers.iter_mut() {
            r.set_mode(vt.mode)?;
            r.render_grid(&vt.grid, rows, cols, fg, bg)?;
            r.flush()?;
        }
        Ok(())
    }

    pub fn to_desc(&self) -> SeatDesc {
        SeatDesc {
            id: self.id,
            name: self.name.clone(),
            active_vt: self.active_vt,
            input_devices: self.input_devices.clone(),
            output_devices: self.output_devices.clone(),
        }
    }
}
