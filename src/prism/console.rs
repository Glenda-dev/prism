use crate::prism::PrismServer;
use core::cmp::min;
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

    pub fn handle_console_poll_read(&mut self, badge: Badge) -> Result<usize, Error> {
        let vt_id = self.active_vt_for_badge(badge)?;
        if let Err(e) = self.poll_input_rings() {
            warn!("poll_input_rings during TERM_POLL_READ failed: {:?}", e);
        }
        let has_input = self
            .muxer
            .vts
            .iter()
            .find(|v| v.id == vt_id)
            .map(|v| !v.input_buffer.is_empty())
            .ok_or(Error::NotFound)?;
        Ok(usize::from(has_input))
    }

    pub fn handle_console_get_str(
        &mut self,
        badge: Badge,
        len: usize,
        utcb: &mut UTCB,
    ) -> Result<usize, Error> {
        let vt_id = self.active_vt_for_badge(badge)?;
        let max_len = core::cmp::min(len, utcb.buffer_mut().len());

        if max_len == 0 {
            utcb.set_size(0);
            return Ok(0);
        }

        loop {
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

            if read_len > 0 {
                utcb.set_size(read_len);
                return Ok(read_len);
            }

            if let Err(e) = self.poll_input_rings() {
                warn!("poll_input_rings during TERM_GET_STR failed: {:?}", e);
            }
            core::hint::spin_loop();
        }
    }

    pub fn handle_terminal_set_mode(&mut self, badge: Badge, mode: usize) -> Result<(), Error> {
        log!("Set display mode req={:#x} for VT {} (requested by {:?})", mode, badge.bits(), badge);
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
        log!("Set display mode {:?} for VT {} (requested by {:?})", mode, vt_id, badge);
        if let Some(vt) = self.muxer.vts.iter_mut().find(|v| v.id == vt_id) {
            vt.set_mode(mode);
            let _ = self.muxer.render_vt(vt_id);
            return Ok(());
        }
        Err(Error::NotFound)
    }

    pub fn handle_terminal_get_winsize(&mut self, badge: Badge) -> Result<WindowSize, Error> {
        let vt_id = self.active_vt_for_badge(badge)?;
        log!("Get window size req for VT {} (requested by {:?})", vt_id, badge);
        self.muxer.vts.iter().find(|v| v.id == vt_id).map(|v| v.winsize).ok_or(Error::NotFound)
    }

    pub fn handle_terminal_set_winsize(
        &mut self,
        badge: Badge,
        winsize: WindowSize,
    ) -> Result<(), Error> {
        let vt_id = self.active_vt_for_badge(badge)?;
        log!("Set window size {:?} for VT {} (requested by {:?})", winsize, vt_id, badge);
        if let Some(vt) = self.muxer.vts.iter_mut().find(|v| v.id == vt_id) {
            vt.set_winsize(winsize);
            let _ = self.muxer.render_vt(vt_id);
            return Ok(());
        }
        Err(Error::NotFound)
    }

    pub fn handle_terminal_get_termios(
        &mut self,
        badge: Badge,
        utcb: &mut UTCB,
    ) -> Result<usize, Error> {
        let vt_id = self.active_vt_for_badge(badge)?;
        let vt = self.muxer.vts.iter_mut().find(|v| v.id == vt_id).ok_or(Error::NotFound)?;
        log!("Get termios req for VT {} (requested by {:?})", vt_id, badge);
        let n = min(utcb.buffer_mut().len(), vt.termios.len());
        utcb.buffer_mut()[..n].copy_from_slice(&vt.termios[..n]);
        utcb.set_size(n);
        Ok(n)
    }

    pub fn handle_terminal_set_termios(
        &mut self,
        badge: Badge,
        req_len: usize,
        utcb: &mut UTCB,
    ) -> Result<(), Error> {
        let vt_id = self.active_vt_for_badge(badge)?;
        let vt = self.muxer.vts.iter_mut().find(|v| v.id == vt_id).ok_or(Error::NotFound)?;
        log!("Set termios req for VT {} (requested by {:?}), len={}", vt_id, badge, req_len);
        let req_len = min(req_len, utcb.buffer().len());
        let n = min(req_len, vt.termios.len());
        vt.termios[..n].copy_from_slice(&utcb.buffer()[..n]);
        Ok(())
    }

    pub fn handle_terminal_get_pgrp(&mut self, badge: Badge) -> Result<usize, Error> {
        let vt_id = self.active_vt_for_badge(badge)?;
        let vt = self.muxer.vts.iter_mut().find(|v| v.id == vt_id).ok_or(Error::NotFound)?;
        log!("Get pgrp req for VT {} (requested by {:?})", vt_id, badge);
        Ok(vt.pgrp as usize)
    }

    pub fn handle_terminal_set_pgrp(&mut self, badge: Badge, pgrp: i32) -> Result<(), Error> {
        let vt_id = self.active_vt_for_badge(badge)?;
        let vt = self.muxer.vts.iter_mut().find(|v| v.id == vt_id).ok_or(Error::NotFound)?;
        log!("Set pgrp req for VT {} (requested by {:?})", vt_id, badge);
        vt.pgrp = pgrp;
        Ok(())
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
        log!(
            "{} exclusive mode for Seat {} (requested by {:?})",
            if exclusive { "Enable" } else { "Disable" },
            seat_id,
            badge
        );
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
