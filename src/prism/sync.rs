use crate::prism::PrismServer;
use crate::prism::device::{DeviceClientKind, DeviceResource};
use crate::renderer::fb::FramebufferRenderer;
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use glenda::cap::{CapPtr, Endpoint, Frame};
use glenda::drivers::client::fb::FbClient;
use glenda::drivers::client::input::InputClient;
use glenda::drivers::client::uart::UartClient;
use glenda::drivers::client::{RingParams, ShmParams};
use glenda::drivers::interface::{DriverClient, FrameBufferDriver};
use glenda::error::Error;
use glenda::interface::{CSpaceService, DeviceService};
use glenda::ipc::Badge;
use glenda::protocol::device::{DeviceQuery, LogicDeviceType};

impl PrismServer<'_> {
    pub fn sync_devices(&mut self) -> Result<(), Error> {
        let mut new_devices = Vec::new();

        log!("Syncing devices...");

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

                        let fb_info = client.get_info();
                        let recv_slot = self.cspace.alloc(self.res_client)?;
                        let fb_vaddr = 0x5600_0000 + self.output_devices.len() * 0x20_0000;
                        match client.setup_shm(
                            self.vspace,
                            self.cspace,
                            self.res_client,
                            fb_vaddr,
                            recv_slot,
                        ) {
                            Ok(shm) => {
                                let width = core::cmp::max(fb_info.width, 1);
                                let height = core::cmp::max(fb_info.height, 1);
                                let bpp = if fb_info.bpp == 0 { 32 } else { fb_info.bpp };
                                let pitch = if fb_info.pitch == 0 {
                                    width.saturating_mul((bpp + 7) / 8)
                                } else {
                                    fb_info.pitch
                                };

                                if let Some(seat0) = self.muxer.seats.iter_mut().find(|s| s.id == 0)
                                {
                                    seat0.renderers.push(Box::new(FramebufferRenderer::new(
                                        shm.as_ptr(),
                                        width,
                                        height,
                                        pitch,
                                        bpp,
                                    )));
                                }

                                let _ = client.set_scanout(shm.paddr());
                            }
                            Err(e) => {
                                log!("Failed to setup FB SHM for {}: {:?}", name, e);
                            }
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
                        log!("Connecting to UART {}...", name);
                        if let Err(e) = self.setup_uart(&name, ep) {
                            log!("Failed to setup UART async: {:?}", e);
                        } else {
                            new_devices.push((name.clone(), LogicDeviceType::Uart));

                            if let Some(seat) = self.muxer.seats.iter_mut().find(|s| s.id == 0) {
                                if !seat.input_devices.contains(&name) {
                                    seat.input_devices.push(name.clone());
                                }
                                if !seat.output_devices.contains(&name) {
                                    seat.output_devices.push(name.clone());
                                }
                                log!("Bound UART {} to Seat 0 as Input/Output", name);
                            }
                        }
                    }
                }
            }
        }

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
                        let notify_ep = self.ipc.endpoint;
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

        for (name, dev_type) in new_devices {
            if let Some(seat) = self.muxer.seats.get_mut(0) {
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

        Ok(())
    }

    fn setup_uart(&mut self, name: &str, ep: Endpoint) -> Result<(), Error> {
        let notify_ep = self.ipc.endpoint;

        let ring_recv_slot = self.cspace.alloc(self.res_client)?;
        let data_recv_slot = self.cspace.alloc(self.res_client)?;

        let ring_params = RingParams {
            sq_entries: 16,
            cq_entries: 16,
            vaddr: 0x5030_0000 + self.input_devices.len() * 0x10_0000,
            size: 4096,
            notify_ep,
            recv_slot: ring_recv_slot,
        };

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

        client.connect(self.vspace, self.cspace)?;

        let _ = client.read_multishot((shm_params.vaddr + 2048) as usize, 1024, 2);

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
