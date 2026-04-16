use crate::prism::PrismServer;
use crate::prism::device::{DeviceClientKind, DeviceResource};
use crate::renderer::fb::FramebufferRenderer;
use alloc::boxed::Box;
use alloc::string::String;
use glenda::arch::mem::PGSIZE;
use glenda::cap::{CSPACE_CAP, CapPtr, Endpoint, Page};
use glenda::drivers::client::fb::FbClient;
use glenda::drivers::client::input::InputClient;
use glenda::drivers::client::uart::UartClient;
use glenda::drivers::client::{RingParams, ShmParams};
use glenda::drivers::interface::{DriverClient, FrameBufferDriver};
use glenda::error::Error;
use glenda::interface::{CSpaceService, DeviceService, VSpaceService};
use glenda::ipc::Badge;
use glenda::protocol::device::LogicDeviceType;

impl PrismServer<'_> {
    fn release_mapping(&mut self, vaddr: usize, pages: usize) {
        if vaddr == 0 || pages == 0 {
            return;
        }
        if let Err(e) = self.vspace.unmap(vaddr, pages) {
            warn!("release_mapping: unmap failed vaddr={:#x} pages={} err={:?}", vaddr, pages, e);
        }
    }

    fn release_slot(&mut self, slot: CapPtr) {
        if slot.is_null() {
            return;
        }
        let _ = CSPACE_CAP.delete(slot);
        self.cspace.free(slot);
    }

    fn release_device_resource(&mut self, mut resource: DeviceResource) {
        match &mut resource.kind {
            DeviceClientKind::Uart(client) => {
                let _ = client.disconnect();
            }
            DeviceClientKind::Fb(client) => {
                let _ = client.disconnect();
            }
            DeviceClientKind::Input(client) => {
                let _ = client.disconnect();
            }
        }

        self.release_mapping(resource.ring_vaddr, resource.ring_pages);
        self.release_mapping(resource.data_vaddr, resource.data_pages);

        self.release_slot(resource.endpoint.cap());
        self.release_slot(resource.ring_frame.cap());
        self.release_slot(resource.data_frame.cap());
    }

    fn is_device_still_bound(&self, device_name: &str) -> bool {
        self.muxer.seats.iter().any(|seat| {
            seat.input_devices.iter().any(|d| d == device_name)
                || seat.output_devices.iter().any(|d| d == device_name)
        })
    }

    fn select_scanout_paddr_for_device(&self, device_name: &str) -> Option<usize> {
        self.muxer.seats.iter().find_map(|seat| seat.first_scanout_paddr_for_device(device_name))
    }

    fn set_fb_scanout(&mut self, device_name: &str, paddr: usize) -> Result<(), Error> {
        let dev = self.output_devices.get_mut(device_name).ok_or(Error::NotFound)?;
        let client = match &mut dev.kind {
            DeviceClientKind::Fb(client) => client,
            _ => return Err(Error::InvalidType),
        };
        client.set_scanout(paddr)
    }

    fn prune_unbound_device(&mut self, device_name: &str) {
        if self.is_device_still_bound(device_name) {
            return;
        }

        let removed_in = self.input_devices.remove(device_name);
        let removed_out = self.output_devices.remove(device_name);
        let removed_any = removed_in.is_some() || removed_out.is_some();

        if let Some(resource) = removed_in {
            self.release_device_resource(resource);
        }
        if let Some(resource) = removed_out {
            self.release_device_resource(resource);
        }
        if removed_any {
            log!(
                "device '{}' is no longer referenced by any seat, resources released",
                device_name
            );
        }
    }

    fn ensure_device_connected(&mut self, device_name: &str) -> Result<(), Error> {
        if self.input_devices.contains_key(device_name)
            || self.output_devices.contains_key(device_name)
        {
            return Ok(());
        }

        let (_, desc) = self.dev_client.get_logic_desc(Badge::null(), device_name)?;
        let slot = self.cspace.alloc(self.res_client)?;
        let ep = self.dev_client.alloc_logic(Badge::null(), desc.dev_type, device_name, slot)?;

        match desc.dev_type {
            LogicDeviceType::Fb => self.setup_fb(device_name, ep),
            LogicDeviceType::Uart => self.setup_uart(device_name, ep),
            LogicDeviceType::Input => self.setup_input(device_name, ep),
            other => {
                error!(
                    "bind_device_to_seat: unsupported logic device '{}' type {:?}",
                    device_name, other
                );
                Err(Error::InvalidType)
            }
        }
    }

    fn setup_fb(&mut self, name: &str, ep: Endpoint) -> Result<(), Error> {
        let mut client = FbClient::new(ep);
        client.connect(self.vspace, self.cspace)?;

        let resources = DeviceResource {
            name: String::from(name),
            kind: DeviceClientKind::Fb(client),
            ring_frame: Page::from(CapPtr::null()),
            ring_vaddr: 0,
            ring_pages: 0,
            data_frame: Page::from(CapPtr::null()),
            data_vaddr: 0,
            data_pages: 0,
            endpoint: ep,
        };
        self.output_devices.insert(String::from(name), resources);
        Ok(())
    }

    fn setup_input(&mut self, name: &str, ep: Endpoint) -> Result<(), Error> {
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

        let mut client = InputClient::new(ep, self.res_client, ring_params, shm_params);
        client.connect(self.vspace, self.cspace)?;

        let resources = DeviceResource {
            name: String::from(name),
            kind: DeviceClientKind::Input(client),
            ring_frame: Page::from(ring_recv_slot),
            ring_vaddr: ring_params.vaddr,
            ring_pages: ring_params.size.div_ceil(PGSIZE),
            data_frame: data_shm.frame().clone(),
            data_vaddr: shm_params.vaddr,
            data_pages: shm_params.size.div_ceil(PGSIZE),
            endpoint: ep,
        };
        self.input_devices.insert(String::from(name), resources);
        Ok(())
    }

    fn attach_fb_renderer_to_seat(
        &mut self,
        seat_id: usize,
        device_name: &str,
    ) -> Result<(), Error> {
        let recv_slot = self.cspace.alloc(self.res_client)?;
        let fb_vaddr = 0x5600_0000 + seat_id * 0x20_0000;

        let (fb_ptr, frame_slot, map_vaddr, map_pages, scanout_paddr, width, height, pitch, bpp) = {
            let device = self.output_devices.get_mut(device_name).ok_or(Error::NotFound)?;
            let client = match &mut device.kind {
                DeviceClientKind::Fb(client) => client,
                _ => return Err(Error::InvalidType),
            };

            let fb_info = client.get_info();
            let shm =
                client.setup_shm(self.vspace, self.cspace, self.res_client, fb_vaddr, recv_slot)?;

            let width = core::cmp::max(fb_info.width, 1);
            let height = core::cmp::max(fb_info.height, 1);
            let bpp = if fb_info.bpp == 0 { 32 } else { fb_info.bpp };
            let pitch = if fb_info.pitch == 0 {
                width.saturating_mul((bpp + 7) / 8)
            } else {
                fb_info.pitch
            };

            let _ = client.set_scanout(shm.paddr());
            (
                shm.as_ptr(),
                recv_slot,
                fb_vaddr,
                shm.size().div_ceil(PGSIZE),
                shm.paddr(),
                width,
                height,
                pitch,
                bpp,
            )
        };

        let seat = self.muxer.seats.iter_mut().find(|s| s.id == seat_id).ok_or(Error::NotFound)?;
        seat.attach_renderer(
            device_name,
            scanout_paddr,
            frame_slot,
            map_vaddr,
            map_pages,
            Box::new(FramebufferRenderer::new(fb_ptr, width, height, pitch, bpp)),
        );
        Ok(())
    }

    pub fn bind_device_to_seat(&mut self, seat_id: usize, device_name: &str) -> Result<(), Error> {
        self.ensure_device_connected(device_name)?;

        let bind_input = self.input_devices.get(device_name).map(|d| d.is_input()).unwrap_or(false)
            || self.output_devices.get(device_name).map(|d| d.is_input()).unwrap_or(false);
        let bind_output =
            self.output_devices.get(device_name).map(|d| d.is_output()).unwrap_or(false)
                || self.input_devices.get(device_name).map(|d| d.is_output()).unwrap_or(false);

        if !bind_input && !bind_output {
            return Err(Error::InvalidType);
        }

        let is_fb = self
            .output_devices
            .get(device_name)
            .map(|d| matches!(&d.kind, DeviceClientKind::Fb(_)))
            .unwrap_or(false);

        let mut attach_fb_renderer = false;
        {
            let seat =
                self.muxer.seats.iter_mut().find(|s| s.id == seat_id).ok_or(Error::NotFound)?;

            let dev_name = String::from(device_name);
            if bind_input && !seat.input_devices.contains(&dev_name) {
                seat.input_devices.push(dev_name.clone());
            }
            if bind_output && !seat.output_devices.contains(&dev_name) {
                seat.output_devices.push(dev_name);
                attach_fb_renderer = is_fb;
            }
        }

        if attach_fb_renderer {
            self.attach_fb_renderer_to_seat(seat_id, device_name)?;
        }

        Ok(())
    }

    pub fn unbind_device_from_seat(
        &mut self,
        seat_id: usize,
        device_name: &str,
    ) -> Result<(), Error> {
        let is_fb_output = self
            .output_devices
            .get(device_name)
            .map(|d| matches!(&d.kind, DeviceClientKind::Fb(_)))
            .unwrap_or(false);

        let seat = self.muxer.seats.iter_mut().find(|s| s.id == seat_id).ok_or(Error::NotFound)?;
        seat.input_devices.retain(|d| d != device_name);
        seat.output_devices.retain(|d| d != device_name);

        let mut fb_detach_cleanups = alloc::vec::Vec::new();
        if is_fb_output {
            fb_detach_cleanups = seat.detach_renderers_for_device(device_name);
        }

        for cleanup in &fb_detach_cleanups {
            self.release_mapping(cleanup.vaddr, cleanup.pages);
            self.release_slot(cleanup.frame_slot);
        }

        if is_fb_output && !fb_detach_cleanups.is_empty() {
            if let Some(next_scanout) = self.select_scanout_paddr_for_device(device_name) {
                let _ = self.set_fb_scanout(device_name, next_scanout);
            } else {
                let _ = self.set_fb_scanout(device_name, 0);
            }
        }

        self.prune_unbound_device(device_name);
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
            ring_frame: Page::from(ring_recv_slot),
            ring_vaddr: ring_params.vaddr,
            ring_pages: ring_params.size.div_ceil(PGSIZE),
            data_frame: data_shm.frame().clone(),
            data_vaddr: shm_params.vaddr,
            data_pages: shm_params.size.div_ceil(PGSIZE),
            endpoint: ep,
        };

        self.input_devices.insert(String::from(name), resource);

        log!("UART {} setup complete", name);
        Ok(())
    }
}
