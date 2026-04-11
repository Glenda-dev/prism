use crate::config::ConfigLoader;
use crate::prism::PrismServer;
use crate::prism::vt::VirtualTerminal;
use glenda::cap::{CSPACE_CAP, CapPtr, Endpoint, Reply, Rights};
use glenda::error::Error;
use glenda::interface::CSpaceService;
use glenda::interface::{DeviceService, InitService, ResourceService, SystemService};
use glenda::ipc::server::{handle_call, handle_cap_call, handle_notify};
use glenda::ipc::{Badge, MsgTag, UTCB};
use glenda::protocol;
use glenda::protocol::device::{HookTarget, LogicDeviceType};
use glenda::protocol::init::ServiceState;
use glenda::protocol::resource::{ResourceType, VT_ENDPOINT};
use psf2_font::TERMINUS_FONT_DATA;

impl SystemService for PrismServer<'_> {
    fn init(&mut self) -> Result<(), Error> {
        log!("Initializing...");
        self.init_client.report_service(Badge::null(), ServiceState::Starting)?;
        // Load configuration and font
        let mut config_loader =
            ConfigLoader::new(&mut self.res_client, &mut self.cspace, &mut self.vspace);
        if let Ok(tty_config) = config_loader.load_tty_config() {
            let font_data = config_loader.load_font(tty_config.font.as_str());
            let font_data_static: &'static [u8] = unsafe { core::mem::transmute(font_data) };
            self.set_font(font_data_static).map_err(|_| Error::MappingFailed)?;
        } else {
            // Loading failed, fallback to default font and config
            let font_data = TERMINUS_FONT_DATA;
            self.set_font(font_data).map_err(|_| Error::MappingFailed)?;
        }

        let hook_ep = self.ipc.endpoint.cap();
        self.dev_client.hook(Badge::null(), HookTarget::Type(LogicDeviceType::Fb), hook_ep)?;
        self.dev_client.hook(Badge::null(), HookTarget::Type(LogicDeviceType::Uart), hook_ep)?;
        self.dev_client.hook(Badge::null(), HookTarget::Type(LogicDeviceType::Input), hook_ep)?;

        Ok(())
    }

    fn listen(&mut self, ep: Endpoint, reply: CapPtr, recv: CapPtr) -> Result<(), Error> {
        self.ipc.endpoint = ep;
        self.ipc.reply = Reply::from(reply);
        self.ipc.recv = recv;
        self.res_client.register_cap(
            Badge::null(),
            ResourceType::Endpoint,
            VT_ENDPOINT,
            ep.cap(),
        )?;
        Ok(())
    }

    fn run(&mut self) -> Result<(), Error> {
        self.ipc.running = true;
        self.init_client.report_service(Badge::null(), ServiceState::Running)?;
        while self.ipc.running {
            let mut utcb = unsafe { UTCB::new() };
            utcb.clear();
            utcb.set_reply_window(self.ipc.reply.cap());
            utcb.set_recv_window(self.ipc.recv);
            match self.ipc.endpoint.recv(&mut utcb) {
                Ok(b) => b,
                Err(e) => {
                    error!("Recv error: {:?}", e);
                    continue;
                }
            };

            let badge = utcb.get_badge();
            let proto = utcb.get_msg_tag().proto();
            let label = utcb.get_msg_tag().label();
            match self.dispatch(&mut utcb) {
                Ok(()) => {}
                Err(e) => {
                    if e == Error::Success {
                        let _ = CSPACE_CAP.delete(self.ipc.reply.cap());
                        continue;
                    }
                    error!(
                        "Failed to dispatch message for {}: {:?}, proto={:#x}, label={:#x}",
                        badge, e, proto, label
                    );
                    utcb.set_msg_tag(MsgTag::err());
                    utcb.set_mr(0, e as usize);
                }
            };
            if let Err(e) = self.reply(utcb) {
                error!("Reply error: {:?}", e);
            }
        }
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

            // Terminal Service (Per-VT)
            (protocol::TERMINAL_PROTO, protocol::terminal::TERM_SET_BAUD) => |_s: &mut Self, u: &mut UTCB| {
                let baud = u.get_mr(0);
                let badge = u.get_badge();
                handle_call(u, |_| {
                    log!("Setting baudrate to {} for VT {}", baud, badge.bits());
                    Ok(0usize)
                })
            },
            (protocol::TERMINAL_PROTO, protocol::terminal::TERM_SET_LCR) => |_s: &mut Self, u: &mut UTCB| {
                let lcr = u.get_mr(0);
                let badge = u.get_badge();
                handle_call(u, |_| {
                    log!("Setting LCR to {:#x} for VT {}", lcr, badge.bits());
                    Ok(0usize)
                })
            },
            (protocol::TERMINAL_PROTO, protocol::terminal::TERM_IOCTL) => |_s: &mut Self, u: &mut UTCB| {
                let req = u.get_mr(0);
                let arg = u.get_mr(1);
                let badge = u.get_badge();
                handle_call(u, |_| {
                    log!("Generic IOCTL req={:#x} arg={:#x} for VT {}", req, arg, badge.bits());
                    Ok(0usize)
                })
            },

            // Terminal VTS protocol
            (protocol::TERMINAL_PROTO, protocol::terminal::VTS_ALLOC_VT) => |s: &mut Self, u: &mut UTCB| {
                let name = unsafe { u.read_str()? };
                handle_cap_call(u, |u| {
                    let vt = VirtualTerminal::new(0, &name);
                    let id = s.muxer.add_vt(vt);

                    // Create an individual endpoint for this VT, badged with VT ID
                    let slot = s.cspace.alloc(s.res_client)?;
                    let badge = Badge::new(id);
                    CSPACE_CAP.mint_self(s.ipc.endpoint.cap(), slot, badge, Rights::ALL)?;

                    log!("Created VT {} ({})", id, name);
                    u.set_mr(0, id);
                    Ok(slot)
                })
            },
            (protocol::TERMINAL_PROTO, protocol::terminal::VTS_SWITCH_VT) => |s: &mut Self, u: &mut UTCB| {
                let seat_id = u.get_mr(0);
                let vt_id = u.get_mr(1);
                handle_call(u, |u| s.switch_vt(u.get_badge(), seat_id, vt_id))
            },
            (protocol::TERMINAL_PROTO, protocol::terminal::VTS_SET_EXCLUSIVE) => |s: &mut Self, u: &mut UTCB| {
                let seat_id = u.get_mr(0);
                let exclusive = u.get_mr(1) != 0;
                handle_call(u, |u| s.set_exclusive(u.get_badge(), seat_id, exclusive))
            },

            (protocol::KERNEL_PROTO, protocol::kernel::NOTIFY) => |s: &mut Self, u: &mut UTCB| {
                let badge_bits = u.get_badge().bits();
                handle_notify(u, |_u| {
                    if badge_bits & glenda::protocol::device::NOTIFY_HOOK != 0 {
                        if let Err(e) = s.sync_devices() {
                            log!("Failed to sync and attach devices: {:?}", e);
                        }
                    }
                    // For UART/InputClient, any notification might mean new data
                    if let Err(e) = s.poll_input_rings() {
                        log!("Failed to poll input rings: {:?}", e);
                    }
                    Ok(())
                })?;
                Err(Error::Success)
            }
        };
        res
    }

    fn reply(&mut self, utcb: &mut UTCB) -> Result<(), Error> {
        self.ipc.reply.reply(utcb)
    }

    fn stop(&mut self) {
        self.ipc.running = false;
        let _ = self.init_client.report_service(Badge::null(), ServiceState::Stopped);
    }
}
