//! Constant and Capability Layout definitions for Prism.

use glenda::cap::{CapPtr, Endpoint, Kernel};

/// Standard Prism Capability Slots.
pub const DEVICE_SLOT: CapPtr = CapPtr::from(9);
pub const INIT_SLOT: CapPtr = CapPtr::from(10);
pub const KERNEL_SLOT: CapPtr = CapPtr::from(11);
pub const TTY_CONFIG_BUF_SLOT: CapPtr = CapPtr::from(12);
pub const TTY_FONT_BUF_SLOT: CapPtr = CapPtr::from(13);

pub const DEVICE_CAP: Endpoint = Endpoint::from(DEVICE_SLOT);
pub const INIT_CAP: Endpoint = Endpoint::from(INIT_SLOT);
pub const KERNEL_CAP: Kernel = Kernel::from(KERNEL_SLOT);

/// JSON config file path in initrd.
pub const TTY_CONFIG_PATH: &str = "tty.json";

/// Default Display Geometry.
pub const DEFAULT_ROWS: u16 = 25;
pub const DEFAULT_COLS: u16 = 80;
pub const DEFAULT_WIDTH: u16 = 800;
pub const DEFAULT_HEIGHT: u16 = 600;

/// Shared Memory Pool Size (1MB).
pub const MEM_POOL_SIZE: usize = 1024 * 1024;

/// Virtual Address Constants.
pub const TTY_CONFIG_VA: usize = 0x5000_0000;
pub const TTY_FONT_VA: usize = 0x5010_0000;
pub const SHM_CLIENT_POOL_VA: usize = 0x5020_0000;
pub const SHM_VA: usize = 0x5030_0000;
