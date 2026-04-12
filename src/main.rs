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
use glenda::client::{DeviceClient, InitClient, ResourceClient};
use glenda::interface::{ResourceService, SystemService};
use glenda::ipc::Badge;
use glenda::protocol::resource::{DEVICE_ENDPOINT, INIT_ENDPOINT, ResourceType};
use glenda::utils::manager::{CSpaceManager, VSpaceManager};
use layout::*;
use prism::PrismServer;

#[unsafe(no_mangle)]
fn main() -> usize {
    glenda::console::init_logging("Prism");
    log!("Prism Virtual Terminal Manager starting...");

    // 1. Setup CSpace
    let mut cspace_mgr = CSpaceManager::new(CSPACE_CAP, 16);
    let mut vspace_mgr = VSpaceManager::new(glenda::cap::VSPACE_CAP, 0x7000_0000, 0x8000_0000);
    let mut res_client = ResourceClient::new(MONITOR_CAP);

    // 2. Obtain Unicorn (Device Manager) Endpoint
    res_client
        .get_cap(Badge::null(), ResourceType::Endpoint, DEVICE_ENDPOINT, DEVICE_SLOT)
        .expect("Failed to get Unicorn endpoint");

    let mut dev_client = DeviceClient::new(DEVICE_CAP);
    let mut res_client = ResourceClient::new(MONITOR_CAP);

    res_client
        .get_cap(Badge::null(), ResourceType::Endpoint, INIT_ENDPOINT, INIT_SLOT)
        .expect("Failed to get init endpoint");

    let mut init_client = InitClient::new(INIT_CAP);

    // 6. Take over Kernel Console
    res_client
        .get_cap(Badge::null(), ResourceType::Kernel, 0, KERNEL_SLOT)
        .expect("Failed to get Kernel cap");

    // Create an endpoint for receiving console and VTS calls
    res_client
        .alloc(Badge::null(), CapType::Endpoint, 0, ENDPOINT_SLOT)
        .expect("Failed to alloc prism endpoint");

    // 3. Setup Prism Server
    let mut server = PrismServer::new(
        &mut dev_client,
        &mut res_client,
        &mut cspace_mgr,
        &mut vspace_mgr,
        &mut init_client,
        KERNEL_CAP,
    );

    // 7. Run Server Loop
    server.listen(ENDPOINT_CAP, REPLY_SLOT, RECV_SLOT).expect("Failed to listen");

    // 4. Initialize Prism Server with font and config
    server.init().expect("Failed to initialize Prism Server");

    server.run().expect("Server loop failed");
    0
}
