use alloc::string::String;
use glenda::cap::{Endpoint, Frame};
use glenda::drivers::client::fb::FbClient;
use glenda::drivers::client::input::InputClient;
use glenda::drivers::client::uart::UartClient;
use glenda::drivers::interface::FrameBufferDriver;
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
    pub ring_vaddr: usize,
    pub ring_pages: usize,
    pub data_frame: Frame,
    pub data_vaddr: usize,
    pub data_pages: usize,
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
                // Try to use TX Ring Buffer first
                let pushed = client.push_tx_ring(bytes);

                if pushed < bytes.len() {
                    warn!("Prism: TX ring full, {} bytes dropped", bytes.len() - pushed);
                    // 在純環化設計中，如果環滿了應該返回 WouldBlock 或者在 client 层等待
                    // 暫時返回 Ok 但記錄警告，或者返回 Error
                    return Err(Error::WouldBlock);
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
