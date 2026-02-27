use crate::layout::*;
use alloc::string::String;
use glenda::client::ResourceClient;
use glenda::error::Error;
use glenda::interface::{MemoryService, ResourceService};
use glenda::ipc::Badge;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct TtyConfig {
    pub font: String,
}

pub struct ConfigLoader<'a> {
    res_client: &'a mut ResourceClient,
}

impl<'a> ConfigLoader<'a> {
    pub fn new(res_client: &'a mut ResourceClient) -> Self {
        Self { res_client }
    }

    pub fn load_tty_config(&mut self) -> Result<TtyConfig, Error> {
        log!("Loading {}...", TTY_CONFIG_PATH);
        let (frame, size) =
            self.res_client.get_config(Badge::null(), TTY_CONFIG_PATH, TTY_CONFIG_BUF_SLOT)?;
        let config_vaddr = TTY_CONFIG_VA;
        self.res_client.mmap(Badge::null(), frame, config_vaddr, size)?;
        let config_data = unsafe { core::slice::from_raw_parts(config_vaddr as *const u8, size) };
        let cfg = serde_json::from_slice(config_data).expect("Failed to parse TTY config");
        Ok(cfg)
    }

    pub fn load_font(&mut self, font_path: &str) -> &'static [u8] {
        if font_path.is_empty() {
            log!("Using built-in terminus font...");
            return psf2_font::TERMINUS_FONT_DATA;
        }

        log!("Loading font {}...", font_path);
        match self.res_client.get_config(Badge::null(), font_path, TTY_FONT_BUF_SLOT) {
            Ok((font_frame, font_size)) => {
                let font_vaddr = TTY_FONT_VA;
                self.res_client
                    .mmap(Badge::null(), font_frame, font_vaddr, font_size)
                    .map(|_| unsafe {
                        core::slice::from_raw_parts(font_vaddr as *const u8, font_size)
                    })
                    .unwrap_or_else(|_| {
                        log!("Failed to map font {}, fall back to built-in terminus", font_path);
                        psf2_font::TERMINUS_FONT_DATA
                    })
            }
            Err(_) => {
                log!("Font {} not found, fall back to built-in terminus", font_path);
                psf2_font::TERMINUS_FONT_DATA
            }
        }
    }
}
