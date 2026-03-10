use alloc::string::String;
use glenda::cap::{Endpoint, Frame};
use glenda::drivers::client::fb::FbClient;
use glenda::drivers::client::input::InputClient;
use glenda::drivers::client::uart::UartClient;
use glenda::drivers::interface::{FrameBufferDriver, UartDriver};
use glenda::error::Error;

/// Unified Device Client types supported by Prism
pub enum DeviceClientKind {
    /// UART device (Input & Output)
    Uart(UartClient),
    /// Framebuffer device (Output)
    Fb(FbClient),
    /// Input device (Input via Client)
    Input(InputClient),
}

/// A template structure to store device resources and control interfaces.
pub struct DeviceResource {
    pub name: String,
    pub kind: DeviceClientKind,
    pub ring_frame: Frame,
    pub data_frame: Frame,
    pub endpoint: Endpoint,
}

impl DeviceResource {
    pub fn is_input(&self) -> bool {
        match &self.kind {
            DeviceClientKind::Uart(_) | DeviceClientKind::Input(_) => true,
            DeviceClientKind::Fb(_) => false,
        }
    }

    pub fn is_output(&self) -> bool {
        match &self.kind {
            DeviceClientKind::Uart(_) | DeviceClientKind::Fb(_) => true,
            DeviceClientKind::Input(_) => false,
        }
    }

    pub fn write_raw(&mut self, bytes: &[u8]) -> Result<(), Error> {
        match &mut self.kind {
            DeviceClientKind::Uart(client) => {
                let shm_params = client.shm_params();
                let shm_vaddr = shm_params.vaddr;
                let shm_size = shm_params.size;

                // Use the first half of SHM for Write (0..2048)
                // The second half (2048..) is reserved for Read in mod.rs
                let write_buf_limit = core::cmp::min(shm_size, 2048);

                if bytes.len() <= write_buf_limit {
                    unsafe {
                        core::ptr::copy_nonoverlapping(
                            bytes.as_ptr(),
                            shm_vaddr as *mut u8,
                            bytes.len(),
                        );
                    }
                    // user_data = 1 refers to WRITE completion
                    client.write_async(shm_vaddr as usize, bytes.len() as u32, 1)?;
                } else {
                    // Fallback to synchronous write if data is too large for the buffer
                    client.write(bytes)?;
                }
                Ok(())
            }
            DeviceClientKind::Fb(_) => Ok(()),
            DeviceClientKind::Input(_) => Err(Error::InvalidType),
        }
    }

    pub fn write_str(&mut self, s: &str) -> Result<(), Error> {
        self.write_raw(s.as_bytes())
    }

    pub fn set_scanout(&mut self, paddr: usize) -> Result<(), Error> {
        match &mut self.kind {
            DeviceClientKind::Fb(client) => client.set_scanout(paddr),
            _ => Err(Error::InvalidType),
        }
    }
}
