use alloc::string::String;
use glenda::cap::{Endpoint, Frame};
use glenda::drivers::client::fb::FbClient;
use glenda::drivers::client::input::InputClient;
use glenda::drivers::client::uart::UartClient;
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
                let shm_vaddr = client.shm_params().vaddr;
                let shm_size = client.shm_params().size;

                if bytes.len() <= shm_size {
                    unsafe {
                        core::ptr::copy_nonoverlapping(
                            bytes.as_ptr(),
                            shm_vaddr as *mut u8,
                            bytes.len(),
                        );
                    }
                    client.write_async(shm_vaddr as u64, bytes.len() as u32, 1)?;
                } else {
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
}
