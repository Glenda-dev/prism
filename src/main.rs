#![no_std]
#![no_main]
#![allow(dead_code)]

#[macro_use]
extern crate glenda;
extern crate alloc;

mod config;
mod layout;
mod prism;
mod renderer;

use glenda::cap::{
    CSPACE_CAP, CapType, ENDPOINT_CAP, ENDPOINT_SLOT, MONITOR_CAP, RECV_SLOT, REPLY_SLOT,
};
use glenda::client::{DeviceClient, ResourceClient};
use glenda::interface::{ResourceService, SystemService};
use glenda::ipc::Badge;
use glenda::protocol::resource::{DEVICE_ENDPOINT, ResourceType};
use glenda::utils::manager::{CSpaceManager, CSpaceService, NullProvider};
use layout::*;
use prism::PrismServer;
use prism::device::DeviceManager;

#[unsafe(no_mangle)]
fn main() -> usize {
    glenda::console::init_logging("Prism");
    log!("Prism Virtual Terminal Manager starting...");

    // 1. Setup CSpace
    let mut cspace_mgr = CSpaceManager::new(CSPACE_CAP, 16);
    let mut res_client = ResourceClient::new(MONITOR_CAP);

    // 2. Obtain Unicorn (Device Manager) Endpoint
    let unicorn_ep_slot = cspace_mgr.alloc(&mut NullProvider).expect("Failed to alloc slot");
    res_client
        .get_cap(Badge::null(), ResourceType::Endpoint, DEVICE_ENDPOINT, unicorn_ep_slot)
        .expect("Failed to get Unicorn endpoint");

    let mut unicorn_client = DeviceClient::new(glenda::cap::Endpoint::from(unicorn_ep_slot));
    let mut res_client = ResourceClient::new(MONITOR_CAP);

    // 6. Take over Kernel Console
    res_client
        .get_cap(Badge::null(), ResourceType::Kernel, 0, KERNEL_SLOT)
        .expect("Failed to get Kernel cap");

    // Create an endpoint for receiving console and VTS calls
    res_client
        .alloc(Badge::null(), CapType::Endpoint, 0, ENDPOINT_SLOT)
        .expect("Failed to alloc prism endpoint");

    // 3. Setup Prism Server
    let device_manager = DeviceManager::new(&mut unicorn_client);
    let mut server = PrismServer::new(device_manager, &mut res_client, &mut cspace_mgr);

    // 7. Run Server Loop
    server.listen(ENDPOINT_CAP, RECV_SLOT, REPLY_SLOT).expect("Failed to listen");

    // 4. Initialize Prism Server with font and config
    server.init().expect("Failed to initialize Prism Server");

    server.run().expect("Server loop failed");

    0
}
