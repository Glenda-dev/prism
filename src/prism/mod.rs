pub mod device;
pub mod mux;
pub mod seat;
pub mod server;
pub mod shm;
pub mod vt;

use crate::prism::device::{DeviceClientKind, DeviceResource};
use crate::prism::mux::Muxer;
use crate::prism::seat::Seat;
use crate::prism::shm::MemoryPool;
use crate::prism::vt::VirtualTerminal;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use glenda::cap::{CapPtr, Endpoint, Frame, Kernel, Reply};
use glenda::client::{DeviceClient, InitClient, ResourceClient};
use glenda::drivers::client::fb::FbClient;
use glenda::drivers::client::input::InputClient;
use glenda::drivers::client::uart::UartClient;
use glenda::drivers::client::{RingParams, ShmParams};
use glenda::drivers::interface::DriverClient;
use glenda::error::Error;
use glenda::interface::{CSpaceService, DeviceService, InitService};
use glenda::ipc::{Badge, UTCB};
use glenda::protocol::device::{DeviceQuery, LogicDeviceType};
use glenda::utils::manager::{CSpaceManager, VSpaceManager};

pub struct PrismServer<'a> {
    pub running: bool,
    pub vt0_ready: bool,
    pub seats: Vec<Seat>,
    pub recv: CapPtr,
    pub vts: Vec<VirtualTerminal>,
    pub muxer: Muxer,
    pub dev_client: &'a mut DeviceClient,

    /// Unified input devices (UARTs, Keyboards, etc.) for high-frequency polling
    pub input_devices: BTreeMap<String, DeviceResource>,
    /// Unified output devices (Framebuffers, etc.)
    pub output_devices: BTreeMap<String, DeviceResource>,

    pub res_client: &'a mut ResourceClient,
    pub mem_pool: MemoryPool,
    pub endpoint: Endpoint,
    pub reply: Reply,

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
            running: false,
            vt0_ready: false,
            seats: Vec::new(),
            vts: Vec::new(),
            muxer: Muxer::new(),
            dev_client,
            input_devices: BTreeMap::new(),
            output_devices: BTreeMap::new(),
            res_client,
            mem_pool: MemoryPool::new(),
            endpoint: Endpoint::from(CapPtr::null()),
            recv: CapPtr::null(),
            reply: Reply::from(CapPtr::null()),
            seat_map: BTreeMap::new(),
            pending_input: BTreeMap::new(),
            cspace,
            vspace,
            init_client,
            kernel_cap,
        };

        // Create Default Seat and VT (System Console)
        server.seats.push(Seat::new(0, "System Seat"));
        server.seats[0].active_vt = Some(0);
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
            // After writing to VT, unifed output via Muxer
            self.muxer.output_to_devices(vt, &s, &mut self.input_devices, &mut self.output_devices);
            self.muxer.render_vt(vt, &mut self.input_devices, &mut self.output_devices);
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

    pub fn switch_vt(&mut self, badge: Badge, seat_id: u32, vt_id: u32) -> Result<(), Error> {
        log!("Switch Seat {} to VT {} (requested by {:?})", seat_id, vt_id, badge);
        if let Some(seat) = self.seats.get_mut(seat_id as usize) {
            if self.vts.iter().any(|v| v.id == vt_id) {
                seat.active_vt = Some(vt_id);
                // Trigger re-render of the whole seat might be needed, but for now we just switch
                return Ok(());
            }
        }
        Err(Error::NotFound)
    }

    pub fn poll_input_rings(&mut self) -> Result<(), Error> {
        // Poll input devices (both Rings where Prism is server and Uarts where Prism is client)
        let mut uarts_to_process = Vec::new();

        for (name, device) in &self.input_devices {
            match &device.kind {
                DeviceClientKind::Uart(client) => {
                    if client.peek_cqe().is_some() {
                        uarts_to_process.push(name.clone());
                    }
                }
                _ => {}
            }
        }

        // Handle UART events
        for name in uarts_to_process {
            self.process_uart_ring(&name)?;
        }

        Ok(())
    }

    fn process_uart_ring(&mut self, name: &str) -> Result<(), Error> {
        // Find which seat this UART belongs to, then find the active VT of that seat
        let mut seat_id = None;
        for (i, seat) in self.seats.iter().enumerate() {
            if seat.input_devices.contains(&String::from(name)) {
                seat_id = Some(i);
                break;
            }
        }

        let vt_id = if let Some(sid) = seat_id {
            self.seats[sid].active_vt.unwrap_or(0)
        } else {
            0 // Fallback to System Console
        };

        // 1. Extract inputs
        let mut inputs = Vec::new();
        if let Some(device) = self.input_devices.get_mut(name) {
            if let DeviceClientKind::Uart(client) = &mut device.kind {
                while let Some(cqe) = client.pop_cqe() {
                    // Check user_data to only process READ completions (user_data == 2)
                    // WRITE completions have user_data == 1
                    if cqe.user_data == 2 && cqe.res > 0 {
                        let vaddr = client.shm_params().vaddr + 2048;
                        let count = cqe.res as usize;
                        // For UART READ, SQEs were queued with len 1 and various offsets (or same vaddr)
                        // Driver completes them one by one.
                        for i in 0..count {
                            inputs.push(unsafe { *((vaddr + i) as *const u8) });
                        }
                        // Re-queue the read
                        let _ = client.read_async(vaddr as u64, 1, 2);
                    }
                }
            }
        }

        // 2. Process UTF-8 and Echo in Prism
        if !inputs.is_empty() {
            if let Some(vt) = self.vts.iter_mut().find(|v| v.id == vt_id) {
                for &b in &inputs {
                    // Primitive support for backspace \x7f or \x08
                    if b == 0x7f || b == 0x08 {
                        if !vt.input_buffer.is_empty() {
                            vt.input_buffer.pop();
                            // Echo backspace sequence
                            let _ = self.muxer.output_to_devices(
                                vt,
                                "\x08 \x08",
                                &mut self.input_devices,
                                &mut self.output_devices,
                            );
                        }
                        continue;
                    }

                    vt.input_buffer.push(b);

                    // Echo back raw byte if it's printable ASCII or part of UTF-8 we'll handle
                    // For now, simple ASCII echo.
                    if b >= 32 && b < 127 {
                        let _ = self.muxer.output_to_devices(
                            vt,
                            core::str::from_utf8(&[b]).unwrap_or(""),
                            &mut self.input_devices,
                            &mut self.output_devices,
                        );
                    } else if b == b'\r' {
                        let _ = self.muxer.output_to_devices(
                            vt,
                            "\n",
                            &mut self.input_devices,
                            &mut self.output_devices,
                        );
                    }
                }
                self.muxer.render_vt(vt, &mut self.input_devices, &mut self.output_devices);
            }
        }

        Ok(())
    }

    fn process_ring(&mut self, _badge: Badge) -> Result<(), Error> {
        // InputRing support removed (Prism now acts primarily as a client for UART/Input)
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

    pub fn sync_devices(&mut self) -> Result<(), Error> {
        let mut new_devices = Vec::new();

        log!("Syncing devices...");

        // 1. Sync Framebuffers (Output)
        let query_fb =
            DeviceQuery { name: None, compatible: Vec::new(), dev_type: Some(LogicDeviceType::Fb) };
        if let Ok(names) = self.dev_client.query(Badge::null(), query_fb) {
            self.output_devices.retain(|name, _| names.contains(name));

            for name in names {
                if !self.output_devices.contains_key(&name) {
                    log!("Found FB device: {}", name);
                    let slot = self.cspace.alloc(self.res_client)?;
                    if let Ok(ep) =
                        self.dev_client.alloc_logic(Badge::null(), LogicDeviceType::Fb, &name, slot)
                    {
                        let mut client = FbClient::new(ep);
                        if let Err(e) = client.connect(self.vspace, self.cspace) {
                            log!("Failed to connect to FB {}: {:?}", name, e);
                            continue;
                        }

                        let resources = DeviceResource {
                            name: name.clone(),
                            kind: DeviceClientKind::Fb(client),
                            ring_frame: Frame::from(CapPtr::null()),
                            data_frame: Frame::from(CapPtr::null()),
                            endpoint: ep,
                        };
                        self.output_devices.insert(name.clone(), resources);
                        new_devices.push((name, LogicDeviceType::Fb));
                    }
                }
            }
        }

        // 2. Sync UARTs (Input & Output)
        let query_uart = DeviceQuery {
            name: None,
            compatible: Vec::new(),
            dev_type: Some(LogicDeviceType::Uart),
        };
        if let Ok(names) = self.dev_client.query(Badge::null(), query_uart) {
            for name in names {
                if !self.input_devices.contains_key(&name) {
                    log!("Found UART device: {}", name);
                    let slot = self.cspace.alloc(self.res_client)?;
                    if let Ok(ep) = self.dev_client.alloc_logic(
                        Badge::null(),
                        LogicDeviceType::Uart,
                        &name,
                        slot,
                    ) {
                        // UARTs are setup via setup_uart which will insert into input_devices
                        log!("Connecting to UART {}...", name);
                        if let Err(e) = self.setup_uart(&name, ep) {
                            log!("Failed to setup UART async: {:?}", e);
                        } else {
                            new_devices.push((name.clone(), LogicDeviceType::Uart));

                            // Also register UART as output device for Seat 0
                            if let Some(res) = self.input_devices.get(&name) {
                                if let DeviceClientKind::Uart(_) = &res.kind {
                                    if let Some(seat) = self.seats.get_mut(0) {
                                        if !seat.output_devices.contains(&name) {
                                            seat.output_devices.push(name.clone());
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // 3. Sync Input devices (Input)
        let query_input = DeviceQuery {
            name: None,
            compatible: Vec::new(),
            dev_type: Some(LogicDeviceType::Input),
        };
        if let Ok(names) = self.dev_client.query(Badge::null(), query_input) {
            for name in names {
                if !self.input_devices.contains_key(&name) {
                    log!("Found Input device: {}", name);
                    let slot = self.cspace.alloc(self.res_client)?;
                    if let Ok(ep) = self.dev_client.alloc_logic(
                        Badge::null(),
                        LogicDeviceType::Input,
                        &name,
                        slot,
                    ) {
                        // Setup Input device via io_uring
                        let notify_ep = self.endpoint;
                        let ring_recv_slot = self.cspace.alloc(self.res_client)?;
                        let data_recv_slot = self.cspace.alloc(self.res_client)?;

                        let ring_params = RingParams {
                            sq_entries: 16,
                            cq_entries: 32,
                            vaddr: 0x5040_0000 + self.input_devices.len() * 0x10_0000,
                            size: 4096,
                            notify_ep,
                            recv_slot: ring_recv_slot,
                        };

                        let data_shm = self.mem_pool.alloc_shm(
                            self.vspace,
                            self.cspace,
                            self.res_client,
                            4096,
                            glenda::mem::pool::ShmType::Regular,
                            data_recv_slot,
                        )?;

                        let shm_params = ShmParams {
                            frame: data_shm.frame().clone(),
                            vaddr: data_shm.as_ptr() as usize,
                            paddr: 0,
                            size: 4096,
                            recv_slot: data_recv_slot,
                        };

                        let mut client =
                            InputClient::new(ep, self.res_client, ring_params, shm_params);

                        if let Err(e) = client.connect(self.vspace, self.cspace) {
                            log!("Failed to connect to Input {}: {:?}", name, e);
                            continue;
                        }

                        let resources = DeviceResource {
                            name: name.clone(),
                            kind: DeviceClientKind::Input(client),
                            ring_frame: Frame::from(ring_recv_slot),
                            data_frame: data_shm.frame().clone(),
                            endpoint: ep,
                        };
                        self.input_devices.insert(name.clone(), resources);
                        new_devices.push((name, LogicDeviceType::Input));
                    }
                }
            }
        }

        // Binding all new devices to Seat 0 (Physical Seat)
        for (name, dev_type) in new_devices {
            if let Some(seat) = self.seats.get_mut(0) {
                match dev_type {
                    LogicDeviceType::Input | LogicDeviceType::Uart => {
                        if !seat.input_devices.contains(&name) {
                            seat.input_devices.push(name.clone());
                        }
                    }
                    LogicDeviceType::Fb => {
                        if !seat.output_devices.contains(&name) {
                            seat.output_devices.push(name.clone());
                        }
                    }
                    _ => {}
                }
            }
        }

        // Check if VT0 is ready to report running
        if !self.vt0_ready {
            let has_output = self.seats.get(0).map_or(false, |s| !s.output_devices.is_empty());
            if has_output {
                if let Some(vt) = self.vts.get_mut(0) {
                    // Clear screen first time we get an output device
                    self.muxer.clear_vt(vt, &mut self.input_devices, &mut self.output_devices);
                    vt.write_str("Glenda OS - System Console Initialized.\n");
                    self.muxer.render_vt(vt, &mut self.input_devices, &mut self.output_devices);

                    // Report Running to init service
                    let _ = self.init_client.report_service(
                        Badge::null(),
                        glenda::protocol::init::ServiceState::Running,
                    );
                    self.vt0_ready = true;
                }
            }
        }

        Ok(())
    }

    fn setup_uart(&mut self, name: &str, ep: Endpoint) -> Result<(), Error> {
        let notify_ep = self.endpoint;

        // Allocate persistent slot for resource transfer from CSpace
        let ring_recv_slot = self.cspace.alloc(self.res_client)?;
        let data_recv_slot = self.cspace.alloc(self.res_client)?;

        // 2. Prepare UartClient parameters
        let ring_params = RingParams {
            sq_entries: 16,
            cq_entries: 16,
            vaddr: 0x5030_0000 + self.input_devices.len() * 0x10_0000,
            size: 4096,
            notify_ep,
            recv_slot: ring_recv_slot,
        };

        // Prepare data shm (READ buffer)
        let data_size = 4096;
        let data_shm = self.mem_pool.alloc_shm(
            self.vspace,
            self.cspace,
            self.res_client,
            data_size,
            glenda::mem::pool::ShmType::Regular,
            data_recv_slot,
        )?;

        let shm_params = ShmParams {
            frame: data_shm.frame().clone(),
            vaddr: data_shm.as_ptr() as usize,
            paddr: 0,
            size: data_size,
            recv_slot: data_recv_slot,
        };

        let mut client = UartClient::new(ep, &mut self.res_client, ring_params, shm_params);

        // 3. Connect (Performs SETUP_RING and SETUP_BUFFER)
        // Ensure recv slots are NOT cleared - they are used by UartClient internally
        // after the call returns.
        client.connect(self.vspace, self.cspace)?;

        // Queue initial read
        let _ = client.read_async((shm_params.vaddr + 2048) as u64, 1, 2);

        let resource = DeviceResource {
            name: String::from(name),
            kind: DeviceClientKind::Uart(client),
            ring_frame: Frame::from(ring_recv_slot),
            data_frame: data_shm.frame().clone(),
            endpoint: ep,
        };

        self.input_devices.insert(String::from(name), resource);

        log!("UART {} setup complete", name);
        Ok(())
    }
}
