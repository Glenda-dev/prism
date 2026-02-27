//! Constant and Capability Layout definitions for Prism.

use glenda::cap::CapPtr;

/// Standard Prism Capability Slots.
pub const DEVICE_SLOT: CapPtr = CapPtr::from(0x11);
pub const KERNEL_SLOT: CapPtr = CapPtr::from(0x12);
pub const TTY_CONFIG_BUF_SLOT: CapPtr = CapPtr::from(0x13);
pub const TTY_FONT_BUF_SLOT: CapPtr = CapPtr::from(0x14);

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
pub const TTY_FONT_VA: usize = 0x6000_0000;
pub const SHM_CLIENT_POOL_VA: usize = 0x8000_0000;
