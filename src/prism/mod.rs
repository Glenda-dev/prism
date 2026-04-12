pub mod console;
pub mod device;
pub mod input;
pub mod mux;
pub mod seat;
pub mod server;
pub mod shm;
pub mod sync;
#[cfg(feature = "utf8")]
pub mod utf8;
pub mod vt;

use crate::prism::device::DeviceResource;
use crate::prism::mux::Muxer;
use crate::prism::seat::Seat;
use crate::prism::shm::MemoryPool;
use crate::prism::vt::VirtualTerminal;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use glenda::cap::{CapPtr, Endpoint, Kernel, Reply};
use glenda::client::{DeviceClient, InitClient, ResourceClient};
use glenda::error::Error;
use glenda::ipc::Badge;
use glenda::utils::manager::{CSpaceManager, VSpaceManager};

pub struct PrismIpc {
    pub endpoint: Endpoint,
    pub reply: Reply,
    pub recv: CapPtr,
    pub running: bool,
}

pub struct PrismServer<'a> {
    pub ipc: PrismIpc,
    pub seats: Vec<Seat<'a>>,
    pub vts: Vec<VirtualTerminal>,
    pub muxer: Muxer<'a>,
    pub dev_client: &'a mut DeviceClient,

    /// Unified input devices (UARTs, Keyboards, etc.) for high-frequency polling
    pub input_devices: BTreeMap<String, DeviceResource>,
    /// Unified output devices (Framebuffers, etc.)
    pub output_devices: BTreeMap<String, DeviceResource>,

    pub res_client: &'a mut ResourceClient,
    pub mem_pool: MemoryPool,
    pub seat_map: BTreeMap<Badge, u32>, // Badge -> Seat ID
    pub pending_input: BTreeMap<String, Vec<u8>>, // Device Name -> Input Buffer
    pub cspace: &'a mut CSpaceManager,
    pub vspace: &'a mut VSpaceManager,
    pub init_client: &'a mut InitClient,
    pub kernel_cap: Kernel,
}

impl<'a> PrismServer<'a> {
    pub fn new(
        dev_client: &'a mut DeviceClient,
        res_client: &'a mut ResourceClient,
        cspace: &'a mut CSpaceManager,
        vspace: &'a mut VSpaceManager,
        init_client: &'a mut InitClient,
        kernel_cap: Kernel,
    ) -> Self {
        let mut server = Self {
            ipc: PrismIpc {
                endpoint: Endpoint::from(CapPtr::null()),
                recv: CapPtr::null(),
                reply: Reply::from(CapPtr::null()),
                running: false,
            },
            seats: Vec::new(),
            vts: Vec::new(),
            muxer: Muxer::new(),
            dev_client,
            input_devices: BTreeMap::new(),
            output_devices: BTreeMap::new(),
            res_client,
            mem_pool: MemoryPool::new(),
            seat_map: BTreeMap::new(),
            pending_input: BTreeMap::new(),
            cspace,
            vspace,
            init_client,
            kernel_cap,
        };

        // Create Default Seat and VT (System Console)
        let vt = VirtualTerminal::new(0, "System Console");
        let vt_id = server.muxer.add_vt(vt);

        let mut seat = Seat::new(0, "System Seat");
        seat.active_vt = Some(vt_id);
        // Ensure Seat 0 has System Console mapped for both input and output
        server.muxer.add_seat(seat);
        server.muxer.bind_vt_to_seat(vt_id, 0);

        server
    }

    pub fn set_font(&mut self, data: &'static [u8]) -> Result<(), Error> {
        self.muxer.load_font(data)
    }
}
