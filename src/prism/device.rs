use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use glenda::cap::Endpoint;
use glenda::client::{DeviceClient, ResourceClient};
use glenda::error::Error;
use glenda::interface::DeviceService;
use glenda::ipc::Badge;
use glenda::protocol::device::{DeviceQuery, LogicDeviceType};
use glenda::utils::manager::{CSpaceManager, CSpaceService};

pub struct DeviceManager<'a> {
    pub unicorn: &'a mut DeviceClient,
    pub fb_devices: BTreeMap<String, Endpoint>,
    pub uart_devices: BTreeMap<String, Endpoint>,
    pub input_devices: BTreeMap<String, Endpoint>,
}

impl<'a> DeviceManager<'a> {
    pub fn new(unicorn: &'a mut DeviceClient) -> Self {
        Self {
            unicorn,
            fb_devices: BTreeMap::new(),
            uart_devices: BTreeMap::new(),
            input_devices: BTreeMap::new(),
        }
    }

    pub fn sync_devices(
        &mut self,
        res_client: &mut ResourceClient,
        cspace: &mut CSpaceManager,
    ) -> Result<(), Error> {
        log!("Syncing devices from Unicorn...");

        // 1. Sync Framebuffers
        let query_fb =
            DeviceQuery { name: None, compatible: Vec::new(), dev_type: Some(LogicDeviceType::Fb) };
        if let Ok(names) = self.unicorn.query(Badge::null(), query_fb) {
            // Remove missing devices
            self.fb_devices.retain(|name, _| names.contains(name));

            for name in names {
                if !self.fb_devices.contains_key(&name) {
                    log!("Found FB device: {}", name);
                    let slot = cspace.alloc(res_client)?;
                    if let Ok(ep) =
                        self.unicorn.alloc_logic(Badge::null(), LogicDeviceType::Fb, &name, slot)
                    {
                        self.fb_devices.insert(name, ep);
                    }
                }
            }
        }

        // 2. Sync UARTs
        let query_uart = DeviceQuery {
            name: None,
            compatible: Vec::new(),
            dev_type: Some(LogicDeviceType::Uart),
        };
        if let Ok(names) = self.unicorn.query(Badge::null(), query_uart) {
            // Remove missing devices
            self.uart_devices.retain(|name, _| names.contains(name));

            for name in names {
                if !self.uart_devices.contains_key(&name) {
                    log!("Found UART device: {}", name);
                    let slot = cspace.alloc(res_client)?;
                    if let Ok(ep) =
                        self.unicorn.alloc_logic(Badge::null(), LogicDeviceType::Uart, &name, slot)
                    {
                        self.uart_devices.insert(name, ep);
                    }
                }
            }
        }

        // 3. Sync Input devices
        let query_input = DeviceQuery {
            name: None,
            compatible: Vec::new(),
            dev_type: Some(LogicDeviceType::Input),
        };
        if let Ok(names) = self.unicorn.query(Badge::null(), query_input) {
            // Remove missing devices
            self.input_devices.retain(|name, _| names.contains(name));

            for name in names {
                if !self.input_devices.contains_key(&name) {
                    log!("Found Input device: {}", name);
                    let slot = cspace.alloc(res_client)?;
                    if let Ok(ep) =
                        self.unicorn.alloc_logic(Badge::null(), LogicDeviceType::Input, &name, slot)
                    {
                        self.input_devices.insert(name, ep);
                    }
                }
            }
        }

        Ok(())
    }
}
