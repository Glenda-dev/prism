use alloc::string::String;
use alloc::vec::Vec;
use glenda::protocol::terminal::SeatDesc;

/// Represents a Seat (Input/Output grouping).
pub struct Seat {
    pub id: u32,
    pub name: String,
    pub active_vt: Option<u32>,
    pub input_devices: Vec<String>,
    pub output_devices: Vec<String>,
}

impl Seat {
    pub fn new(id: u32, name: &str) -> Self {
        Self {
            id,
            name: String::from(name),
            active_vt: None,
            input_devices: Vec::new(),
            output_devices: Vec::new(),
        }
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
