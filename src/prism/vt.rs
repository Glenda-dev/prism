use crate::prism::PrismServer;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use glenda::cap::{CapPtr, Endpoint, Frame};
use glenda::error::Error;
use glenda::interface::VirtualTerminalService;
use glenda::ipc::Badge;
use glenda::protocol::terminal::{TerminalDisplayMode, VTDesc, WindowSize};

/// Logic for a Virtual Terminal (VT).
pub struct VirtualTerminal {
    pub id: u32,
    pub name: String,
    pub mode: TerminalDisplayMode,
    pub winsize: WindowSize,
    pub cursor: (u16, u16),
    pub grid: Vec<u32>, // Grid of Unicode chars (32-bit codepoints)
    pub fb_frame: Option<Frame>,
    pub endpoint: Option<Endpoint>,
    pub seat_ids: Vec<u32>,
    pub input_buffer: Vec<u8>,
}

impl VirtualTerminal {
    pub fn new(id: u32, name: &str) -> Self {
        let cols = 80;
        let rows = 25;
        Self {
            id,
            name: String::from(name),
            mode: TerminalDisplayMode::Text,
            winsize: WindowSize { rows, cols, xpixel: 800, ypixel: 600 },
            cursor: (0, 0),
            grid: vec![b' ' as u32; (cols * rows) as usize],
            fb_frame: None,
            endpoint: None,
            seat_ids: Vec::new(),
            input_buffer: Vec::new(),
        }
    }

    pub fn write_str(&mut self, s: &str) {
        for c in s.chars() {
            match c {
                '\n' => {
                    self.cursor.1 += 1;
                    self.cursor.0 = 0;
                }
                '\r' => {
                    self.cursor.0 = 0;
                }
                _ => {
                    let idx = (self.cursor.1 * self.winsize.cols + self.cursor.0) as usize;
                    if idx < self.grid.len() {
                        self.grid[idx] = c as u32;
                    }
                    self.cursor.0 += 1;
                }
            }
            if self.cursor.0 >= self.winsize.cols {
                self.cursor.0 = 0;
                self.cursor.1 += 1;
            }
            if self.cursor.1 >= self.winsize.rows {
                self.scroll_up();
            }
        }
    }

    fn scroll_up(&mut self) {
        let cols = self.winsize.cols as usize;
        let rows = self.winsize.rows as usize;
        for r in 0..rows - 1 {
            for c in 0..cols {
                self.grid[r * cols + c] = self.grid[(r + 1) * cols + c];
            }
        }
        for c in 0..cols {
            self.grid[(rows - 1) * cols + c] = b' ' as u32;
        }
        self.cursor.1 = (rows - 1) as u16;
    }

    pub fn read_char(&mut self) -> Option<u8> {
        if !self.input_buffer.is_empty() { Some(self.input_buffer.remove(0)) } else { None }
    }

    pub fn to_desc(&self) -> VTDesc {
        VTDesc {
            id: self.id,
            name: self.name.clone(),
            mode: self.mode,
            seat_ids: self.seat_ids.clone(),
        }
    }
}

impl VirtualTerminalService for PrismServer<'_> {
    fn create_vt(
        &mut self,
        _badge: Badge,
        name: &str,
        _recv: CapPtr,
    ) -> Result<(u32, Endpoint), Error> {
        let id = self.vts.len() as u32;
        let vt = VirtualTerminal::new(id, name);
        self.vts.push(vt);
        log!("Created VT {} ({})", id, name);
        // In a real implementation, we would create a per-VT endpoint and grant it back
        Ok((id, Endpoint::from(CapPtr::null())))
    }

    fn destroy_vt(&mut self, _badge: Badge, vt_id: u32) -> Result<(), Error> {
        self.vts.retain(|v| v.id != vt_id);
        Ok(())
    }

    fn list_vts(
        &mut self,
        _badge: Badge,
    ) -> Result<Vec<glenda::protocol::terminal::VTDesc>, Error> {
        Ok(self.vts.iter().map(|v| v.to_desc()).collect())
    }

    fn list_seats(
        &mut self,
        _badge: Badge,
    ) -> Result<Vec<glenda::protocol::terminal::SeatDesc>, Error> {
        Ok(self.seats.iter().map(|s| s.to_desc()).collect())
    }

    fn switch_vt(&mut self, _badge: Badge, seat_id: u32, vt_id: u32) -> Result<(), Error> {
        if let Some(seat) = self.seats.iter_mut().find(|s| s.id == seat_id) {
            seat.active_vt = Some(vt_id);
            log!("Switched seat {} to VT {}", seat_id, vt_id);
            Ok(())
        } else {
            Err(Error::NotFound)
        }
    }

    fn bind_seat(&mut self, _badge: Badge, seat_id: u32, vt_id: u32) -> Result<(), Error> {
        if let Some(vt) = self.vts.iter_mut().find(|v| v.id == vt_id) {
            vt.seat_ids.push(seat_id);
            Ok(())
        } else {
            Err(Error::NotFound)
        }
    }

    fn assign_device_to_seat(
        &mut self,
        _badge: Badge,
        seat_id: u32,
        device_name: &str,
    ) -> Result<(), Error> {
        if let Some(seat) = self.seats.iter_mut().find(|s| s.id == seat_id) {
            seat.input_devices.push(alloc::string::String::from(device_name));
            Ok(())
        } else {
            Err(Error::NotFound)
        }
    }

    fn revoke_device_from_seat(
        &mut self,
        _badge: Badge,
        seat_id: u32,
        device_name: &str,
    ) -> Result<(), Error> {
        if let Some(seat) = self.seats.iter_mut().find(|s| s.id == seat_id) {
            seat.input_devices.retain(|d| d != device_name);
            Ok(())
        } else {
            Err(Error::NotFound)
        }
    }
}
