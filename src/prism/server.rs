use crate::config::ConfigLoader;
use crate::prism::PrismServer;
use crate::prism::seat::Seat;
use crate::prism::shm::ShmType;
use crate::prism::vt::VirtualTerminal;
use glenda::cap::{CapPtr, Endpoint, Reply};
use glenda::error::Error;
use glenda::interface::{DeviceService, SystemService, VirtualTerminalService};
use glenda::io::uring::IoUringServer;
use glenda::ipc::server::handle_call;
use glenda::ipc::{Badge, MsgTag, UTCB};
use glenda::protocol;
use glenda::protocol::device::{HookTarget, LogicDeviceType};
use psf2_font::TERMINUS_FONT_DATA;

impl SystemService for PrismServer<'_> {
    fn init(&mut self) -> Result<(), Error> {
        log!("Initializing...");

        // Load configuration and font
        let mut config_loader = ConfigLoader::new(&mut self.res_client);
        if let Ok(tty_config) = config_loader.load_tty_config() {
            let font_data = config_loader.load_font(tty_config.font.as_str());
            self.set_font(font_data).map_err(|_| Error::MappingFailed)?;
        } else {
            // Loading failed, fallback to default font and config
            let font_data = TERMINUS_FONT_DATA;
            self.set_font(font_data).map_err(|_| Error::MappingFailed)?;
        }

        let hook_ep = self.endpoint.cap();
        self.device_manager.unicorn.hook(
            Badge::null(),
            HookTarget::Type(LogicDeviceType::Fb),
            hook_ep,
        )?;
        self.device_manager.unicorn.hook(
            Badge::null(),
            HookTarget::Type(LogicDeviceType::Uart),
            hook_ep,
        )?;
        self.device_manager.unicorn.hook(
            Badge::null(),
            HookTarget::Type(LogicDeviceType::Input),
            hook_ep,
        )?;

        // Initial sync for already registered devices
        if let Err(e) = self.device_manager.sync_devices(self.res_client, self.cspace) {
            log!("Initial device sync failed: {:?}", e);
        }

        // Setup initial seat and default VT
        if self.seats.is_empty() {
            self.seats.push(Seat::new(0, "null"));
            self.seats[0].active_vt = Some(0);
        }

        log!("Initialized with System Console VT");
        Ok(())
    }

    fn listen(&mut self, ep: Endpoint, reply: CapPtr, _recv: CapPtr) -> Result<(), Error> {
        self.endpoint = ep;
        self.reply = reply;
        Ok(())
    }

    fn run(&mut self) -> Result<(), Error> {
        loop {
            let mut utcb = unsafe { UTCB::new() };
            utcb.clear();

            match self.endpoint.recv(&mut utcb) {
                Ok(_) => {}
                Err(e) => {
                    error!("Recv error: {:?}", e);
                    continue;
                }
            }

            match self.dispatch(&mut utcb) {
                Ok(()) => {
                    let _ = self.reply(&mut utcb);
                }
                Err(Error::Success) => {
                    // Handled notification, skip reply
                }
                Err(e) => {
                    let badge = utcb.get_badge();
                    let tag = utcb.get_msg_tag();
                    log!(
                        "Dispatch error: {:?} badge={}, proto={:#x}, label={:#x}",
                        e,
                        badge,
                        tag.proto(),
                        tag.label()
                    );
                    utcb.set_msg_tag(MsgTag::err());
                    utcb.set_mr(0, e as usize);
                    let _ = self.reply(&mut utcb);
                }
            }

            // Reply to caller
            let _ = Reply::from(self.reply).reply(&mut utcb);
        }
        #[allow(unreachable_code)]
        Ok(())
    }

    fn dispatch(&mut self, utcb: &mut UTCB) -> Result<(), Error> {
        let res = glenda::ipc_dispatch! {
            self, utcb,
            // Handle raw Console protocol (forwarded from kernel sys_invoke)
            (protocol::TERMINAL_PROTO, protocol::terminal::TERM_PUT_STR) => |s: &mut Self, u: &mut UTCB| {
                handle_call(u, |u| s.handle_console_put_str(u))
            },
            (protocol::TERMINAL_PROTO, protocol::terminal::TERM_GET_CHAR) => |s: &mut Self, u: &mut UTCB| {
                handle_call(u, |_| s.handle_console_get_char())
            },

            // Input protocol with io_uring support
            (protocol::INPUT_PROTO, protocol::input::SETUP_URING) => |s: &mut Self, u: &mut UTCB| {
                let badge = u.get_badge();
                let entries = u.get_mr(0) as u32;
                handle_call(u, |u| {
                    let recv = u.get_recv_window();
                    let size = (entries as usize * 80 + 4095) & !4095;
                    let shm = s.mem_pool.alloc_shm(&mut s.res_client, size, ShmType::Regular, recv)?;

                    use glenda::io::uring::IoUringBuffer;
                    let ring = unsafe { IoUringBuffer::new(shm.as_ptr(), size, entries, entries) };
                    let ring_server = IoUringServer::new(ring);

                    s.input_rings.insert(badge, (ring_server, shm));
                    // Default associate with Seat 0 for now
                    s.seat_map.insert(badge, 0);

                    Ok(shm.frame().cap().bits())
                })
            },

            // Handle InputEvent (potentially pushed from unicorn or other drivers)
            (protocol::INPUT_PROTO, protocol::input::READ_EVENT) => |_s, u: &mut UTCB| {
                // Return a single event from default seat for now
                handle_call(u, |_| {
                     // TODO: implement classic poll
                     Ok(0usize)
                })
            },

            // Terminal VTS protocol
            (protocol::TERMINAL_PROTO, protocol::terminal::VTS_ALLOC_VT) => |s: &mut Self, u: &mut UTCB| {
                let name = unsafe { u.read_str()? };
                handle_call(u, |_| {
                    let id = s.vts.len() as u32;
                    let vt = VirtualTerminal::new(id, &name);
                    s.vts.push(vt);
                    log!("Created VT {} ({})", id, name);
                    Ok(id as usize)
                })
            },
            (protocol::TERMINAL_PROTO, protocol::terminal::VTS_SWITCH_VT) => |s: &mut Self, u: &mut UTCB| {
                let seat_id = u.get_mr(0) as u32;
                let vt_id = u.get_mr(1) as u32;
                handle_call(u, |u| s.switch_vt(u.get_badge(), seat_id, vt_id))
            },

            (protocol::KERNEL_PROTO, protocol::kernel::NOTIFY) => |s: &mut Self, u: &mut UTCB| {
                use glenda::io::uring::NOTIFY_IO_URING_SQ;
                use glenda::ipc::server::handle_notify;
                let badge_bits = u.get_badge().bits();
                handle_notify(u, |_u| {
                    if badge_bits & glenda::protocol::device::NOTIFY_HOOK != 0 {
                        let _ = s.device_manager.sync_devices(s.res_client, s.cspace);
                    }
                    if badge_bits & NOTIFY_IO_URING_SQ != 0 {
                        let _ = s.poll_input_rings();
                    }
                    Ok(())
                })?;
                Err(Error::Success)
            }
        };
        res
    }

    fn reply(&mut self, _utcb: &mut UTCB) -> Result<(), Error> {
        Ok(())
    }

    fn stop(&mut self) {}
}
