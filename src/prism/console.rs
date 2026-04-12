use crate::prism::PrismServer;
use glenda::error::Error;
use glenda::ipc::{Badge, UTCB};
use glenda::protocol::terminal::{TerminalDisplayMode, WindowSize};

impl PrismServer<'_> {
    fn active_vt_for_badge(&self, badge: Badge) -> Result<usize, Error> {
        let badge_vt = badge.bits();
        if badge_vt != 0 && self.muxer.vts.iter().any(|v| v.id == badge_vt) {
            return Ok(badge_vt);
        }

        let vt_id = self.muxer.seats.first().and_then(|s| s.active_vt).ok_or(Error::NotFound)?;
        if self.muxer.vts.iter().any(|v| v.id == vt_id) { Ok(vt_id) } else { Err(Error::NotFound) }
    }

    pub fn handle_console_put_str(&mut self, utcb: &mut UTCB) -> Result<usize, Error> {
        let s = unsafe { utcb.read_str()? };
        let vt_id = self.active_vt_for_badge(utcb.get_badge())?;

        let vt = self.muxer.vts.iter_mut().find(|v| v.id == vt_id).ok_or(Error::NotFound)?;
        vt.write_str(&s);

        let _ = self.muxer.output_to_devices(
            vt_id,
            s.as_bytes(),
            &mut self.input_devices,
            &mut self.output_devices,
        );
        let _ = self.muxer.render_vt(vt_id);
        Ok(s.len())
    }

    pub fn handle_console_get_char(&mut self, badge: Badge) -> Result<usize, Error> {
        let vt_id = self.active_vt_for_badge(badge)?;
        let vt = self.muxer.vts.iter_mut().find(|v| v.id == vt_id).ok_or(Error::NotFound)?;
        Ok(vt.read_char().unwrap_or(0) as usize)
    }

    pub fn handle_console_get_str(
        &mut self,
        badge: Badge,
        len: usize,
        utcb: &mut UTCB,
    ) -> Result<usize, Error> {
        let vt_id = self.active_vt_for_badge(badge)?;
        let max_len = core::cmp::min(len, utcb.buffer_mut().len());

        let vt = self.muxer.vts.iter_mut().find(|v| v.id == vt_id).ok_or(Error::NotFound)?;
        let mut read_len = 0usize;
        while read_len < max_len {
            if let Some(c) = vt.read_char() {
                utcb.buffer_mut()[read_len] = c;
                read_len += 1;
            } else {
                break;
            }
        }
        utcb.set_size(read_len);
        Ok(read_len)
    }

    pub fn handle_terminal_set_mode(&mut self, badge: Badge, mode: usize) -> Result<(), Error> {
        let display_mode = match mode {
            0 => TerminalDisplayMode::Text,
            1 => TerminalDisplayMode::Graphics,
            2 => TerminalDisplayMode::Bridge,
            _ => return Err(Error::InvalidArgs),
        };
        self.handle_terminal_set_display_mode(badge, display_mode)
    }

    pub fn handle_terminal_set_display_mode(
        &mut self,
        badge: Badge,
        mode: TerminalDisplayMode,
    ) -> Result<(), Error> {
        let vt_id = self.active_vt_for_badge(badge)?;
        if let Some(vt) = self.muxer.vts.iter_mut().find(|v| v.id == vt_id) {
            vt.set_mode(mode);
            let _ = self.muxer.render_vt(vt_id);
            return Ok(());
        }
        Err(Error::NotFound)
    }

    pub fn handle_terminal_get_winsize(&mut self, badge: Badge) -> Result<WindowSize, Error> {
        let vt_id = self.active_vt_for_badge(badge)?;
        self.muxer.vts.iter().find(|v| v.id == vt_id).map(|v| v.winsize).ok_or(Error::NotFound)
    }

    pub fn handle_terminal_set_winsize(
        &mut self,
        badge: Badge,
        winsize: WindowSize,
    ) -> Result<(), Error> {
        let vt_id = self.active_vt_for_badge(badge)?;
        if let Some(vt) = self.muxer.vts.iter_mut().find(|v| v.id == vt_id) {
            vt.set_winsize(winsize);
            let _ = self.muxer.render_vt(vt_id);
            return Ok(());
        }
        Err(Error::NotFound)
    }

    pub fn switch_vt(&mut self, badge: Badge, seat_id: usize, vt_id: usize) -> Result<(), Error> {
        log!("Switch Seat {} to VT {} (requested by {:?})", seat_id, vt_id, badge);
        if let Some(seat) = self.muxer.seats.get_mut(seat_id) {
            if self.muxer.vts.iter().any(|v| v.id == vt_id) {
                seat.active_vt = Some(vt_id);
                return Ok(());
            }
        }
        Err(Error::NotFound)
    }

    pub fn set_exclusive(
        &mut self,
        badge: Badge,
        seat_id: usize,
        exclusive: bool,
    ) -> Result<(), Error> {
        if let Some(seat) = self.muxer.seats.get_mut(seat_id) {
            if exclusive {
                seat.exclusive_owner = Some(badge);
            } else {
                seat.exclusive_owner = None;
            }
            Ok(())
        } else {
            Err(Error::NotFound)
        }
    }
}
