use crate::prism::PrismServer;
use crate::prism::device::DeviceClientKind;
use alloc::collections::BTreeSet;
use alloc::string::String;
use alloc::vec::Vec;
use glenda::drivers::interface::InputDriver;
use glenda::drivers::protocol::input::{
    EV_KEY, InputEvent as DriverInputEvent, KEY_ENTER, KEY_ESC, KEY_SPACE,
};
use glenda::error::Error;
use glenda::ipc::Badge;

const KEY_BACKSPACE: u16 = 14;
const KEY_1: u16 = 2;
const KEY_0: u16 = 11;
const KEY_Q: u16 = 16;
const KEY_P: u16 = 25;
const KEY_A: u16 = 30;
const KEY_L: u16 = 38;
const KEY_Z: u16 = 44;
const KEY_M: u16 = 50;

impl PrismServer<'_> {
    fn apply_input_to_vt(&mut self, vt_id: usize, inputs: &[u8]) -> Option<usize> {
        if inputs.is_empty() {
            return None;
        }

        let (active_vt_id, echo_buf) =
            if let Some(vt) = self.muxer.vts.iter_mut().find(|v| v.id == vt_id) {
                (vt.id, vt.process_input_bytes(inputs))
            } else {
                return None;
            };

        if !echo_buf.is_empty() {
            let _ = self.muxer.output_to_devices(
                active_vt_id,
                &echo_buf,
                &mut self.input_devices,
                &mut self.output_devices,
            );
        }
        Some(active_vt_id)
    }

    fn map_input_event_to_byte(event: &DriverInputEvent) -> Option<u8> {
        if event.type_ != EV_KEY || event.value != 1 {
            return None;
        }

        match event.code {
            KEY_ENTER => Some(b'\r'),
            KEY_SPACE => Some(b' '),
            KEY_ESC => Some(0x1b),
            KEY_BACKSPACE => Some(0x08),
            code => Self::keycode_to_ascii(code),
        }
    }

    fn keycode_to_ascii(code: u16) -> Option<u8> {
        if (KEY_1..=KEY_0).contains(&code) {
            return Some(match code {
                KEY_1 => b'1',
                3 => b'2',
                4 => b'3',
                5 => b'4',
                6 => b'5',
                7 => b'6',
                8 => b'7',
                9 => b'8',
                10 => b'9',
                KEY_0 => b'0',
                _ => return None,
            });
        }

        if (KEY_Q..=KEY_P).contains(&code) {
            return Some(match code {
                16 => b'q',
                17 => b'w',
                18 => b'e',
                19 => b'r',
                20 => b't',
                21 => b'y',
                22 => b'u',
                23 => b'i',
                24 => b'o',
                KEY_P => b'p',
                _ => return None,
            });
        }

        if (KEY_A..=KEY_L).contains(&code) {
            return Some(match code {
                KEY_A => b'a',
                31 => b's',
                32 => b'd',
                33 => b'f',
                34 => b'g',
                35 => b'h',
                36 => b'j',
                37 => b'k',
                KEY_L => b'l',
                _ => return None,
            });
        }

        if (KEY_Z..=KEY_M).contains(&code) {
            return Some(match code {
                KEY_Z => b'z',
                45 => b'x',
                46 => b'c',
                47 => b'v',
                48 => b'b',
                49 => b'n',
                KEY_M => b'm',
                _ => return None,
            });
        }

        None
    }

    pub fn poll_input_rings(&mut self) -> Result<(), Error> {
        let mut uarts_to_process = Vec::new();
        let mut inputs_to_process = Vec::new();
        let mut vts_to_render = BTreeSet::new();

        for (name, device) in &self.input_devices {
            match &device.kind {
                DeviceClientKind::Uart(client) => {
                    if client.peek_cqe().is_some() {
                        uarts_to_process.push(name.clone());
                    }
                }
                DeviceClientKind::Input(client) => {
                    if client.peek_cqe().is_some() {
                        inputs_to_process.push(name.clone());
                    }
                }
                _ => {}
            }
        }

        for name in uarts_to_process {
            if let Some(vt_id) = self.process_uart_ring(&name)? {
                vts_to_render.insert(vt_id);
            }
        }

        for name in inputs_to_process {
            if let Some(vt_id) = self.process_input_ring(&name)? {
                vts_to_render.insert(vt_id);
            }
        }

        for vt_id in vts_to_render {
            let _ = self.muxer.render_vt(vt_id);
        }

        Ok(())
    }

    fn process_uart_ring(&mut self, name: &str) -> Result<Option<usize>, Error> {
        let Some(sid) = self
            .muxer
            .seats
            .iter()
            .find(|seat| seat.input_devices.contains(&String::from(name)))
            .map(|seat| seat.id)
        else {
            // Device has pending input but is not bound to any seat.
            return Ok(None);
        };

        let Some(vt_id) = self.muxer.seats.iter().find(|s| s.id == sid).and_then(|s| s.active_vt)
        else {
            // Seat exists but currently has no active VT.
            return Ok(None);
        };

        let mut inputs = Vec::new();
        if let Some(device) = self.input_devices.get_mut(name) {
            if let DeviceClientKind::Uart(client) = &mut device.kind {
                while let Some(cqe) = client.pop_cqe() {
                    if cqe.user_data == 2 {
                        let mut buf = [0u8; 1024];
                        let read = client.pop_shm_ring(&mut buf);
                        if read > 0 {
                            inputs.extend_from_slice(&buf[..read]);
                        }
                    }
                }
            }
        }

        if !inputs.is_empty() {
            return Ok(self.apply_input_to_vt(vt_id, &inputs));
        }

        Ok(None)
    }

    fn process_input_ring(&mut self, name: &str) -> Result<Option<usize>, Error> {
        let Some(sid) = self
            .muxer
            .seats
            .iter()
            .find(|seat| seat.input_devices.contains(&String::from(name)))
            .map(|seat| seat.id)
        else {
            // Device has pending input but is not bound to any seat.
            return Ok(None);
        };

        let Some(vt_id) = self.muxer.seats.iter().find(|s| s.id == sid).and_then(|s| s.active_vt)
        else {
            // Seat exists but currently has no active VT.
            return Ok(None);
        };

        let mut inputs = Vec::new();
        if let Some(device) = self.input_devices.get_mut(name)
            && let DeviceClientKind::Input(client) = &mut device.kind
        {
            while let Some(event) = client.poll_event() {
                if let Some(b) = Self::map_input_event_to_byte(&event) {
                    inputs.push(b);
                }
            }
        }

        Ok(self.apply_input_to_vt(vt_id, &inputs))
    }

    fn process_ring(&mut self, _badge: Badge) -> Result<(), Error> {
        Ok(())
    }

    fn route_input(&mut self, seat_id: usize, event: glenda::protocol::input::InputEvent) {
        if let Some(seat) = self.muxer.seats.get_mut(seat_id) {
            if let Some(vt_id) = seat.active_vt {
                if let Some(vt) = self.muxer.vts.iter_mut().find(|v| v.id == vt_id) {
                    if event.kind == 1 {
                        if event.value == 1 {
                            if event.code < 256 {
                                vt.input_buffer.push(event.code as u8);
                            }
                        }
                    }
                }
            }
        }
    }
}
