use crate::prism::vt::VirtualTerminal;
use crate::renderer::Renderer;
use alloc::string::String;
use alloc::vec::Vec;
use glenda::cap::CapPtr;
use glenda::error::Error;
use glenda::ipc::Badge;
use glenda::protocol::terminal::SeatDesc;

pub struct RendererDetachCleanup {
    pub frame_slot: CapPtr,
    pub vaddr: usize,
    pub pages: usize,
}

/// Represents a Seat (Input/Output grouping).
pub struct Seat<'a> {
    pub id: usize,
    pub name: String,
    pub active_vt: Option<usize>,
    pub input_devices: Vec<String>,
    pub output_devices: Vec<String>,
    pub renderers: Vec<alloc::boxed::Box<dyn Renderer + 'a>>,
    pub renderer_devices: Vec<String>,
    pub renderer_scanout_paddrs: Vec<usize>,
    pub renderer_frame_slots: Vec<CapPtr>,
    pub renderer_vaddrs: Vec<usize>,
    pub renderer_pages: Vec<usize>,
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
            renderer_devices: Vec::new(),
            renderer_scanout_paddrs: Vec::new(),
            renderer_frame_slots: Vec::new(),
            renderer_vaddrs: Vec::new(),
            renderer_pages: Vec::new(),
            exclusive_owner: None,
        }
    }

    pub fn attach_renderer(
        &mut self,
        device_name: &str,
        scanout_paddr: usize,
        frame_slot: CapPtr,
        vaddr: usize,
        pages: usize,
        renderer: alloc::boxed::Box<dyn Renderer + 'a>,
    ) {
        self.renderers.push(renderer);
        self.renderer_devices.push(String::from(device_name));
        self.renderer_scanout_paddrs.push(scanout_paddr);
        self.renderer_frame_slots.push(frame_slot);
        self.renderer_vaddrs.push(vaddr);
        self.renderer_pages.push(pages);
    }

    pub fn detach_renderers_for_device(&mut self, device_name: &str) -> Vec<RendererDetachCleanup> {
        let mut cleanups = Vec::new();
        let mut idx = 0;
        while idx < self.renderer_devices.len() && idx < self.renderers.len() {
            if self.renderer_devices[idx] == device_name {
                let frame_slot =
                    self.renderer_frame_slots.get(idx).copied().unwrap_or(CapPtr::null());
                let vaddr = self.renderer_vaddrs.get(idx).copied().unwrap_or(0);
                let pages = self.renderer_pages.get(idx).copied().unwrap_or(0);

                self.renderer_devices.remove(idx);
                self.renderers.remove(idx);
                if idx < self.renderer_scanout_paddrs.len() {
                    self.renderer_scanout_paddrs.remove(idx);
                }
                if idx < self.renderer_frame_slots.len() {
                    self.renderer_frame_slots.remove(idx);
                }
                if idx < self.renderer_vaddrs.len() {
                    self.renderer_vaddrs.remove(idx);
                }
                if idx < self.renderer_pages.len() {
                    self.renderer_pages.remove(idx);
                }

                cleanups.push(RendererDetachCleanup { frame_slot, vaddr, pages });
            } else {
                idx += 1;
            }
        }
        cleanups
    }

    pub fn first_scanout_paddr_for_device(&self, device_name: &str) -> Option<usize> {
        self.renderer_devices.iter().enumerate().find_map(|(idx, dev)| {
            if dev == device_name { self.renderer_scanout_paddrs.get(idx).copied() } else { None }
        })
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
