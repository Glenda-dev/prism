pub mod device;
pub mod mux;
pub mod seat;
pub mod server;
pub mod shm;
pub mod vt;

use crate::prism::device::DeviceManager;
use crate::prism::mux::Muxer;
use crate::prism::seat::Seat;
use crate::prism::shm::{MemoryPool, SharedMemory};
use crate::prism::vt::VirtualTerminal;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use glenda::cap::{CapPtr, Endpoint};
use glenda::client::ResourceClient;
use glenda::error::Error;
use glenda::io::uring::IoUringServer;
use glenda::ipc::{Badge, UTCB};
use glenda::utils::manager::CSpaceManager;

/// The Prism Virtual Terminal Server.
pub struct PrismServer<'a> {
    pub seats: Vec<Seat>,
    pub vts: Vec<VirtualTerminal>,
    pub muxer: Muxer,
    pub device_manager: DeviceManager<'a>,
    pub res_client: &'a mut ResourceClient,
    pub mem_pool: MemoryPool,
    pub endpoint: Endpoint,
    pub reply: CapPtr,
    pub input_rings: BTreeMap<Badge, (IoUringServer, SharedMemory)>,
    pub seat_map: BTreeMap<Badge, u32>, // Badge -> Seat ID
    pub pending_input: BTreeMap<String, Vec<u8>>, // Device Name -> Input Buffer
    pub cspace: &'a mut CSpaceManager,
}

impl<'a> PrismServer<'a> {
    pub fn new(
        device_manager: DeviceManager<'a>,
        res_client: &'a mut ResourceClient,
        cspace: &'a mut CSpaceManager,
    ) -> Self {
        let mut server = Self {
            seats: Vec::new(),
            vts: Vec::new(),
            muxer: Muxer::new(),
            device_manager,
            res_client,
            mem_pool: MemoryPool::new(),
            endpoint: Endpoint::from(CapPtr::null()),
            reply: CapPtr::null(),
            input_rings: BTreeMap::new(),
            seat_map: BTreeMap::new(),
            pending_input: BTreeMap::new(),
            cspace,
        };

        // Create Default VT (System Console)
        server.vts.push(VirtualTerminal::new(0, "System Console"));
        server
    }

    pub fn set_font(&mut self, data: &'static [u8]) -> Result<(), ()> {
        if let Some(renderer) = self.muxer.renderer.as_mut() {
            renderer.load_font(data)?;
        }
        Ok(())
    }

    pub fn handle_console_put_str(&mut self, utcb: &mut UTCB) -> Result<usize, Error> {
        let s = unsafe { utcb.read_str()? };
        // Output to default VT (System Console)
        if let Some(vt) = self.vts.get_mut(0) {
            vt.write_str(&s);
            // After writing to VT, we should trigger a re-render
            self.muxer.render_vt(vt);
        }
        Ok(s.len())
    }

    pub fn handle_console_get_char(&mut self) -> Result<usize, Error> {
        // Return char from default seat's active VT
        if let Some(seat) = self.seats.get(0) {
            if let Some(vt_id) = seat.active_vt {
                if let Some(vt) = self.vts.iter_mut().find(|v| v.id == vt_id) {
                    return Ok(vt.read_char().unwrap_or(0) as usize);
                }
            }
        }
        Ok(0)
    }

    pub fn poll_input_rings(&mut self) -> Result<(), Error> {
        let mut badges_to_process = Vec::new();
        for (badge, (server, _)) in &self.input_rings {
            if server.ring.sq_len() > 0 {
                badges_to_process.push(*badge);
            }
        }

        for badge in badges_to_process {
            self.process_ring(badge)?;
        }
        Ok(())
    }

    fn process_ring(&mut self, badge: Badge) -> Result<(), Error> {
        let seat_id = *self.seat_map.get(&badge).unwrap_or(&0) as usize;

        // 1. Collect events from ring
        let mut events = Vec::new();
        {
            let (server, shm) = self.input_rings.get_mut(&badge).ok_or(Error::NotFound)?;
            while let Some(sqe) = server.next_request() {
                if sqe.opcode == glenda::protocol::input::INPUT_OP_READ {
                    let event_ptr = sqe.addr as *const glenda::protocol::input::InputEvent;
                    if shm.contains_ptr(event_ptr as *const u8) {
                        let event = unsafe { *event_ptr };
                        events.push((sqe.user_data, Ok(event)));
                    } else {
                        events.push((sqe.user_data, Err(Error::InvalidArgs)));
                    }
                } else {
                    events.push((sqe.user_data, Err(Error::NotImplemented)));
                }
            }
        }

        // 2. Route events and complete
        for (user_data, res) in events {
            let complete_res = match res {
                Ok(event) => {
                    self.route_input(seat_id, event);
                    1i32
                }
                Err(e) => -(e as i32),
            };

            // Get server again to complete
            if let Some((server, _)) = self.input_rings.get_mut(&badge) {
                server.complete(user_data, complete_res)?;
            }
        }

        Ok(())
    }

    fn route_input(&mut self, seat_id: usize, event: glenda::protocol::input::InputEvent) {
        if let Some(seat) = self.seats.get_mut(seat_id) {
            if let Some(vt_id) = seat.active_vt {
                if let Some(vt) = self.vts.iter_mut().find(|v| v.id == vt_id) {
                    // Routing logic:
                    // If it's character input (simplified mapping for now)
                    if event.kind == 1 {
                        // Key Event
                        if event.value == 1 {
                            // Keydown
                            // Forward code to VT's input buffer
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
