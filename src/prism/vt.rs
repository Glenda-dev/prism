use crate::prism::PrismServer;
#[cfg(feature = "utf8")]
use crate::prism::utf8::Utf8Decoder;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use glenda::cap::{CapPtr, Endpoint};
use glenda::error::Error;
use glenda::interface::VirtualTerminalService;
use glenda::ipc::Badge;
use glenda::protocol::terminal::{TerminalDisplayMode, VTDesc, WindowSize};

/// Logic for a Virtual Terminal (VT).
pub struct VirtualTerminal {
    pub id: usize,
    pub name: String,
    pub mode: TerminalDisplayMode,
    pub winsize: WindowSize,
    pub cursor: (u16, u16),
    pub grid: Vec<u32>, // Grid of Unicode chars (32-bit codepoints)
    pub seat_ids: Vec<usize>,
    pub input_buffer: Vec<u8>,
    pub termios: [u8; 44],
    pub pgrp: i32,
    #[cfg(feature = "utf8")]
    pub decoder: Utf8Decoder,
    pub paddr: usize,
    pub vaddr: *mut u8,
}

impl VirtualTerminal {
    const TERMIOS_LFLAG_OFFSET: usize = 12;
    const TERMIOS_ECHO: u32 = 0x0000_0008;

    pub fn new(id: usize, name: &str) -> Self {
        let cols = 80;
        let rows = 25;
        Self {
            id,
            name: String::from(name),
            mode: TerminalDisplayMode::Text,
            winsize: WindowSize { rows, cols, xpixel: 800, ypixel: 600 },
            cursor: (0, 0),
            grid: vec![b' ' as u32; (cols * rows) as usize],
            seat_ids: Vec::new(),
            input_buffer: Vec::new(),
            termios: [0; 44],
            pgrp: 0,
            #[cfg(feature = "utf8")]
            decoder: Utf8Decoder::new(),
            paddr: 0,
            vaddr: core::ptr::null_mut(),
        }
    }

    pub fn set_buffer(&mut self, paddr: usize, vaddr: *mut u8) {
        self.paddr = paddr;
        self.vaddr = vaddr;
    }

    pub fn process_input_bytes(&mut self, bytes: &[u8]) -> Vec<u8> {
        let mut echo_buf = Vec::new();
        let do_echo = self.local_echo_enabled();
        for &b in bytes {
            if b == 0x7f || b == 0x08 {
                if !self.input_buffer.is_empty() {
                    self.input_buffer.pop();
                    if do_echo {
                        self.write_str("\x08 \x08");
                        echo_buf.extend_from_slice(b"\x08 \x08");
                    }
                }
            } else if b == 0x1b {
                // Escape character (ESC)
                self.input_buffer.push(b);
                // self.write_str("^["); // Visual echo for ESC
                // echo_buf.extend_from_slice(b"^[");
            } else if b == b'\r' {
                self.input_buffer.push(b'\n');
                if do_echo {
                    self.write_str("\n");
                    echo_buf.extend_from_slice(b"\r\n");
                }
            } else {
                #[cfg(feature = "utf8")]
                {
                    let decoded = self.decoder.process_byte(b);
                    if !decoded.is_empty() {
                        for &ub in decoded.as_bytes() {
                            self.input_buffer.push(ub);
                        }
                        if do_echo {
                            self.write_str(&decoded);
                            echo_buf.extend_from_slice(decoded.as_bytes());
                        }
                    }
                }
                #[cfg(not(feature = "utf8"))]
                {
                    self.input_buffer.push(b);
                    if do_echo && b >= 32 && b < 127 {
                        self.write_str(core::str::from_utf8(&[b]).unwrap());
                        echo_buf.push(b);
                    }
                }
            }
        }
        echo_buf
    }

    fn local_echo_enabled(&self) -> bool {
        let off = Self::TERMIOS_LFLAG_OFFSET;
        if self.termios.len() < off + 4 {
            return true;
        }
        let mut raw = [0u8; 4];
        raw.copy_from_slice(&self.termios[off..off + 4]);
        let lflag = u32::from_ne_bytes(raw);
        (lflag & Self::TERMIOS_ECHO) != 0
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

    pub fn set_mode(&mut self, mode: TerminalDisplayMode) {
        self.mode = mode;
    }

    pub fn set_winsize(&mut self, size: WindowSize) {
        let rows = core::cmp::max(size.rows, 1);
        let cols = core::cmp::max(size.cols, 1);

        if rows == self.winsize.rows && cols == self.winsize.cols {
            self.winsize = size;
            return;
        }

        let old_rows = self.winsize.rows as usize;
        let old_cols = self.winsize.cols as usize;
        let new_rows = rows as usize;
        let new_cols = cols as usize;

        let mut new_grid = vec![b' ' as u32; new_rows * new_cols];

        let copy_rows = core::cmp::min(old_rows, new_rows);
        let copy_cols = core::cmp::min(old_cols, new_cols);
        for r in 0..copy_rows {
            let old_base = r * old_cols;
            let new_base = r * new_cols;
            new_grid[new_base..new_base + copy_cols]
                .copy_from_slice(&self.grid[old_base..old_base + copy_cols]);
        }

        self.grid = new_grid;
        self.winsize = WindowSize { rows, cols, xpixel: size.xpixel, ypixel: size.ypixel };
        self.cursor.0 = core::cmp::min(self.cursor.0, cols.saturating_sub(1));
        self.cursor.1 = core::cmp::min(self.cursor.1, rows.saturating_sub(1));
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
    ) -> Result<(usize, Endpoint), Error> {
        let vt = VirtualTerminal::new(0, name);
        let id = self.muxer.add_vt(vt);
        log!("Created VT {} ({})", id, name);
        // In a real implementation, we would create a per-VT endpoint and grant it back
        Ok((id, Endpoint::from(CapPtr::null())))
    }

    fn destroy_vt(&mut self, _badge: Badge, vt_id: usize) -> Result<(), Error> {
        self.muxer.vts.retain(|v| v.id != vt_id);
        Ok(())
    }

    fn list_vts(
        &mut self,
        _badge: Badge,
    ) -> Result<Vec<glenda::protocol::terminal::VTDesc>, Error> {
        Ok(self.muxer.vts.iter().map(|v| v.to_desc()).collect())
    }

    fn list_seats(
        &mut self,
        _badge: Badge,
    ) -> Result<Vec<glenda::protocol::terminal::SeatDesc>, Error> {
        Ok(self.muxer.seats.iter().map(|s| s.to_desc()).collect())
    }

    fn switch_vt(&mut self, _badge: Badge, seat_id: usize, vt_id: usize) -> Result<(), Error> {
        if let Some(seat) = self.muxer.seats.iter_mut().find(|s| s.id == seat_id) {
            seat.active_vt = Some(vt_id);
            log!("Switched seat {} to VT {}", seat_id, vt_id);
            Ok(())
        } else {
            Err(Error::NotFound)
        }
    }

    fn bind_seat(&mut self, _badge: Badge, seat_id: usize, vt_id: usize) -> Result<(), Error> {
        if let Some(vt) = self.muxer.vts.iter_mut().find(|v| v.id == vt_id) {
            vt.seat_ids.push(seat_id);
            Ok(())
        } else {
            Err(Error::NotFound)
        }
    }

    fn open_vt(&mut self, _badge: Badge, vt_id: usize, _recv: CapPtr) -> Result<Endpoint, Error> {
        if self.muxer.vts.iter().any(|v| v.id == vt_id) {
            Ok(self.ipc.endpoint)
        } else {
            Err(Error::NotFound)
        }
    }

    fn get_pty_lock(&mut self, _badge: Badge, vt_id: usize) -> Result<bool, Error> {
        if !self.muxer.vts.iter().any(|v| v.id == vt_id) {
            return Err(Error::NotFound);
        }
        Ok(*self.pty_locks.get(&vt_id).unwrap_or(&true))
    }

    fn set_pty_lock(&mut self, _badge: Badge, vt_id: usize, locked: bool) -> Result<(), Error> {
        if !self.muxer.vts.iter().any(|v| v.id == vt_id) {
            return Err(Error::NotFound);
        }
        self.pty_locks.insert(vt_id, locked);
        Ok(())
    }

    fn assign_device_to_seat(
        &mut self,
        _badge: Badge,
        seat_id: usize,
        device_name: &str,
    ) -> Result<(), Error> {
        if let Some(seat) = self.muxer.seats.iter_mut().find(|s| s.id == seat_id) {
            seat.input_devices.push(alloc::string::String::from(device_name));
            Ok(())
        } else {
            Err(Error::NotFound)
        }
    }

    fn revoke_device_from_seat(
        &mut self,
        _badge: Badge,
        seat_id: usize,
        device_name: &str,
    ) -> Result<(), Error> {
        if let Some(seat) = self.muxer.seats.iter_mut().find(|s| s.id == seat_id) {
            seat.input_devices.retain(|d| d != device_name);
            Ok(())
        } else {
            Err(Error::NotFound)
        }
    }
}
