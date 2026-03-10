use crate::prism::device::DeviceResource;
use crate::prism::seat::Seat;
use crate::prism::vt::VirtualTerminal;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use glenda::error::Error;

/// Multiplexer for terminal input routing and output mapping.
pub struct Muxer<'a> {
    pub vts: Vec<VirtualTerminal>,
    pub seats: Vec<Seat<'a>>,
    pub vt_to_seats: BTreeMap<usize, Vec<usize>>, // VT ID -> List of Seat IDs
    next_vt_id: usize,
    next_seat_id: usize,
}

impl<'a> Muxer<'a> {
    pub fn new() -> Self {
        Self {
            vts: Vec::new(),
            seats: Vec::new(),
            vt_to_seats: BTreeMap::new(),
            next_vt_id: 0,
            next_seat_id: 0,
        }
    }

    /// Dispatches output to all devices associated with the VT.
    /// If a device is a UART, it sends the raw bytes (ANSI stream).
    pub fn output_to_devices(
        &mut self,
        vt_id: usize,
        data: &[u8],
        input_devices: &mut BTreeMap<String, DeviceResource>,
        output_devices: &mut BTreeMap<String, DeviceResource>,
    ) -> Result<(), Error> {
        if let Some(seat_ids) = self.vt_to_seats.get(&vt_id) {
            for &sid in seat_ids {
                if let Some(seat) = self.seats.iter().find(|s| s.id == sid) {
                    // Only output if the seat is actively viewing this VT
                    if seat.output_devices.is_empty() {
                        warn!(
                            "No output devices bound to Seat {} (attempted to write to VT {})",
                            sid, vt_id
                        );
                    }

                    for dev_name in &seat.output_devices {
                        // Search in both maps (UARTs are often in input_devices due to setup)
                        if let Some(dev) = output_devices.get_mut(dev_name) {
                            let _ = dev.write_raw(data);
                        } else if let Some(dev) = input_devices.get_mut(dev_name) {
                            let _ = dev.write_raw(data);
                        } else {
                            warn!("Output device {} bound to Seat {} was not found", dev_name, sid);
                        }
                    }
                }
            }
        }
        Ok(())
    }

    pub fn add_vt(&mut self, mut vt: VirtualTerminal) -> usize {
        let id = self.next_vt_id;
        vt.id = id;
        self.vts.push(vt);
        self.next_vt_id += 1;
        id
    }

    pub fn add_seat(&mut self, mut seat: Seat<'a>) -> usize {
        let id = self.next_seat_id;
        seat.id = id;
        self.seats.push(seat);
        self.next_seat_id += 1;
        id
    }

    pub fn bind_vt_to_seat(&mut self, vt_id: usize, seat_id: usize) {
        if let Some(seat) = self.seats.iter_mut().find(|s| s.id == seat_id) {
            seat.active_vt = Some(vt_id);
            self.vt_to_seats.entry(vt_id).or_insert_with(Vec::new).push(seat_id);
        }
    }

    pub fn load_font(&mut self, data: &'static [u8]) -> Result<(), Error> {
        for seat in self.seats.iter_mut() {
            for r in seat.renderers.iter_mut() {
                r.load_font(data)?;
            }
        }
        Ok(())
    }

    pub fn clear_vt(&mut self, vt_id: usize) -> Result<(), Error> {
        let vt = self.vts.iter_mut().find(|v| v.id == vt_id).ok_or(Error::NotFound)?;
        vt.grid.fill(b' ' as u32);
        vt.cursor = (0, 0);

        // Seat renderers clear
        if let Some(seat_ids) = self.vt_to_seats.get(&vt_id) {
            for &sid in seat_ids {
                if let Some(seat) = self.seats.iter_mut().find(|s| s.id == sid) {
                    if seat.active_vt == Some(vt_id) {
                        for r in seat.renderers.iter_mut() {
                            r.clear()?;
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Mix/Compose outputs for current display.
    pub fn render_vt(&mut self, vt_id: usize) -> Result<(), Error> {
        let vt = self.vts.iter().find(|v| v.id == vt_id).ok_or(Error::NotFound)?;

        // Delegate to mapped seats
        if let Some(seat_ids) = self.vt_to_seats.get(&vt_id) {
            for &sid in seat_ids {
                if let Some(seat) = self.seats.iter_mut().find(|s| s.id == sid) {
                    if seat.active_vt == Some(vt_id) {
                        seat.render(vt)?;
                    }
                }
            }
        }
        Ok(())
    }
}
