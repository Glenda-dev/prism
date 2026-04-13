use crate::config::ConfigLoader;
use crate::prism::PrismServer;
use crate::prism::vt::VirtualTerminal;
use glenda::cap::{CSPACE_CAP, CapPtr, Endpoint, Reply, Rights};
use glenda::error::Error;
use glenda::interface::CSpaceService;
use glenda::interface::{InitService, ResourceService, SystemService, VSpaceService};
use glenda::ipc::server::{handle_call, handle_cap_call, handle_notify};
use glenda::ipc::{Badge, MsgFlags, MsgTag, UTCB};
use glenda::protocol;
use glenda::protocol::init::ServiceState;
use glenda::protocol::resource::{ResourceType, VT_ENDPOINT};
use glenda::protocol::terminal::{TerminalDisplayMode, TerminalUringConfig};
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

        // One-shot initial scan/setup for already existing devices.
        self.sync_devices()?;

        self.init_client.report_service(Badge::null(), ServiceState::Running)?;

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
            (protocol::TERMINAL_PROTO, protocol::terminal::TERM_GET_STR) => |s: &mut Self, u: &mut UTCB| {
                let len = u.get_mr(0);
                handle_call(u, |u| s.handle_console_get_str(u.get_badge(), len, u))
            },
            (protocol::TERMINAL_PROTO, protocol::terminal::TERM_GET_CHAR) => |s: &mut Self, u: &mut UTCB| {
                handle_call(u, |u| s.handle_console_get_char(u.get_badge()))
            },
            (protocol::TERMINAL_PROTO, protocol::terminal::TERM_POLL_READ) => |s: &mut Self, u: &mut UTCB| {
                handle_call(u, |u| s.handle_console_poll_read(u.get_badge()))
            },
            (protocol::TERMINAL_PROTO, protocol::terminal::TERM_PUT_CHAR) => |s: &mut Self, u: &mut UTCB| {
                let ch = u.get_mr(0) as u8;
                handle_call(u, |u| {
                    let mut tmp = [0u8; 1];
                    tmp[0] = ch;
                    let len = u.write(&tmp);
                    s.handle_console_put_str(u)?;
                    Ok(len)
                })
            },
            (protocol::TERMINAL_PROTO, protocol::terminal::TERM_GET_URING) => |s: &mut Self, u: &mut UTCB| {
                let frame_slot = s.cspace.alloc(s.res_client)?;
                s.res_client.alloc(Badge::null(), glenda::cap::CapType::Frame, 1, frame_slot)?;
                let frame = glenda::cap::Frame::from(frame_slot);
                let ring_vaddr = s.mem_pool.reserve_vaddr_dma(4096);
                s.vspace.map_frame(
                    frame,
                    ring_vaddr,
                    glenda::mem::Perms::READ | glenda::mem::Perms::WRITE,
                    1,
                    s.res_client,
                    s.cspace,
                )?;
                let _ = unsafe {
                    glenda::io::uring::IoUringBuffer::new(ring_vaddr as *mut u8, 4096, 16, 16)
                };

                let cfg = TerminalUringConfig {
                    size: 4096,
                    sq_entries: 16,
                    cq_entries: 16,
                    sq_offset: glenda::io::uring::HEADER_SIZE,
                    cq_offset: glenda::io::uring::HEADER_SIZE + (16 * glenda::io::uring::SQE_SIZE),
                    buf_offset: glenda::io::uring::HEADER_SIZE
                        + (16 * glenda::io::uring::SQE_SIZE)
                        + (16 * glenda::io::uring::CQE_SIZE),
                };

                unsafe { u.write_postcard(&cfg)?; }
                u.set_cap_transfer(frame_slot);
                u.set_msg_tag(MsgTag::new(
                    protocol::GENERIC_PROTO,
                    protocol::generic::REPLY,
                    MsgFlags::OK | MsgFlags::HAS_CAP | MsgFlags::HAS_BUFFER,
                ));
                Ok(())
            },
            (protocol::TERMINAL_PROTO, protocol::terminal::TERM_SET_MODE) => |s: &mut Self, u: &mut UTCB| {
                let mode = u.get_mr(0);
                handle_call(u, |u| {
                    s.handle_terminal_set_mode(u.get_badge(), mode)?;
                    Ok(0usize)
                })
            },
            (protocol::TERMINAL_PROTO, protocol::terminal::TERM_SET_DISPLAY) => |s: &mut Self, u: &mut UTCB| {
                let mode = unsafe { u.read_postcard::<TerminalDisplayMode>()? };
                handle_call(u, |u| {
                    s.handle_terminal_set_display_mode(u.get_badge(), mode)?;
                    Ok(0usize)
                })
            },
            (protocol::TERMINAL_PROTO, protocol::terminal::TERM_GET_WINSIZE) => |s: &mut Self, u: &mut UTCB| {
                handle_call(u, |u| {
                    let winsize = s.handle_terminal_get_winsize(u.get_badge())?;
                    unsafe { u.write_postcard(&winsize)?; }
                    Ok(0usize)
                })
            },
            (protocol::TERMINAL_PROTO, protocol::terminal::TERM_SET_WINSIZE) => |s: &mut Self, u: &mut UTCB| {
                let winsize = unsafe { u.read_postcard()? };
                handle_call(u, |u| {
                    s.handle_terminal_set_winsize(u.get_badge(), winsize)?;
                    Ok(0usize)
                })
            },
            (protocol::TERMINAL_PROTO, protocol::terminal::TERM_GET_TERMIOS) => |s: &mut Self, u: &mut UTCB| {
                handle_call(u, |u| {
                    s.handle_terminal_get_termios(u.get_badge(), u)?;
                    Ok(0usize)
                })
            },
            (protocol::TERMINAL_PROTO, protocol::terminal::TERM_SET_TERMIOS) => |s: &mut Self, u: &mut UTCB| {
                let len = u.get_mr(0);
                handle_call(u, |u| {
                    s.handle_terminal_set_termios(u.get_badge(), len, u)?;
                    Ok(0usize)
                })
            },
            (protocol::TERMINAL_PROTO, protocol::terminal::TERM_GET_PGRP) => |s: &mut Self, u: &mut UTCB| {
                handle_call(u, |u| s.handle_terminal_get_pgrp(u.get_badge()))
            },
            (protocol::TERMINAL_PROTO, protocol::terminal::TERM_SET_PGRP) => |s: &mut Self, u: &mut UTCB| {
                let pgrp = u.get_mr(0) as i32;
                handle_call(u, |u| {
                    s.handle_terminal_set_pgrp(u.get_badge(), pgrp)?;
                    Ok(0usize)
                })
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
            // Terminal VTS protocol
            (protocol::TERMINAL_PROTO, protocol::terminal::VTS_ALLOC_VT) => |s: &mut Self, u: &mut UTCB| {
                let name = unsafe { u.read_str()? };
                handle_cap_call(u, |u| {
                    let vt = VirtualTerminal::new(0, &name);
                    let id = s.muxer.add_vt(vt);
                    s.pty_locks.insert(id, true);

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
            (protocol::TERMINAL_PROTO, protocol::terminal::VTS_FREE_VT) => |s: &mut Self, u: &mut UTCB| {
                let vt_id = u.get_mr(0);
                handle_call(u, |_u| {
                    if !s.muxer.vts.iter().any(|v| v.id == vt_id) {
                        return Err(Error::NotFound);
                    }
                    s.muxer.vts.retain(|v| v.id != vt_id);
                    s.muxer.vt_to_seats.remove(&vt_id);
                    for seat in s.muxer.seats.iter_mut() {
                        if seat.active_vt == Some(vt_id) {
                            seat.active_vt = None;
                        }
                    }
                    s.pty_locks.remove(&vt_id);
                    Ok(0usize)
                })
            },
            (protocol::TERMINAL_PROTO, protocol::terminal::VTS_OPEN_VT) => |s: &mut Self, u: &mut UTCB| {
                let vt_id = u.get_mr(0);
                handle_cap_call(u, |_u| {
                    if !s.muxer.vts.iter().any(|v| v.id == vt_id) {
                        return Err(Error::NotFound);
                    }
                    let slot = s.cspace.alloc(s.res_client)?;
                    CSPACE_CAP.mint_self(s.ipc.endpoint.cap(), slot, Badge::new(vt_id), Rights::ALL)?;
                    Ok(slot)
                })
            },
            (protocol::TERMINAL_PROTO, protocol::terminal::VTS_GET_PTY_LOCK) => |s: &mut Self, u: &mut UTCB| {
                let vt_id = u.get_mr(0);
                handle_call(u, |_u| {
                    if !s.muxer.vts.iter().any(|v| v.id == vt_id) {
                        return Err(Error::NotFound);
                    }
                    Ok(usize::from(*s.pty_locks.get(&vt_id).unwrap_or(&true)))
                })
            },
            (protocol::TERMINAL_PROTO, protocol::terminal::VTS_SET_PTY_LOCK) => |s: &mut Self, u: &mut UTCB| {
                let vt_id = u.get_mr(0);
                let locked = u.get_mr(1) != 0;
                handle_call(u, |_u| {
                    if !s.muxer.vts.iter().any(|v| v.id == vt_id) {
                        return Err(Error::NotFound);
                    }
                    s.pty_locks.insert(vt_id, locked);
                    Ok(0usize)
                })
            },
            (protocol::TERMINAL_PROTO, protocol::terminal::VTS_LIST_VTS) => |s: &mut Self, u: &mut UTCB| {
                handle_call(u, |u| {
                    let list: alloc::vec::Vec<_> = s.muxer.vts.iter().map(|v| v.to_desc()).collect();
                    unsafe { u.write_postcard(&list)?; }
                    Ok(0usize)
                })
            },
            (protocol::TERMINAL_PROTO, protocol::terminal::VTS_LIST_SEATS) => |s: &mut Self, u: &mut UTCB| {
                handle_call(u, |u| {
                    let list: alloc::vec::Vec<_> = s.muxer.seats.iter().map(|seat| seat.to_desc()).collect();
                    unsafe { u.write_postcard(&list)?; }
                    Ok(0usize)
                })
            },
            (protocol::TERMINAL_PROTO, protocol::terminal::VTS_BIND_SEAT) => |s: &mut Self, u: &mut UTCB| {
                let seat_id = u.get_mr(0);
                let vt_id = u.get_mr(1);
                handle_call(u, |_u| {
                    if !s.muxer.vts.iter().any(|v| v.id == vt_id) {
                        return Err(Error::NotFound);
                    }
                    if !s.muxer.seats.iter().any(|seat| seat.id == seat_id) {
                        return Err(Error::NotFound);
                    }
                    s.muxer.bind_vt_to_seat(vt_id, seat_id);
                    Ok(0usize)
                })
            },
            (protocol::TERMINAL_PROTO, protocol::terminal::SEAT_BIND_DEVICE) => |s: &mut Self, u: &mut UTCB| {
                let seat_id = u.get_mr(0);
                let dev_name = unsafe { u.read_str()? };
                handle_call(u, |_u| {
                    s.bind_device_to_seat(seat_id, &dev_name)?;
                    Ok(0usize)
                })
            },
            (protocol::TERMINAL_PROTO, protocol::terminal::SEAT_UNBIND_DEVICE) => |s: &mut Self, u: &mut UTCB| {
                let seat_id = u.get_mr(0);
                let dev_name = unsafe { u.read_str()? };
                handle_call(u, |_u| {
                    s.unbind_device_from_seat(seat_id, &dev_name)?;
                    Ok(0usize)
                })
            },
            (protocol::TERMINAL_PROTO, protocol::terminal::VTS_SET_EXCLUSIVE) => |s: &mut Self, u: &mut UTCB| {
                let seat_id = u.get_mr(0);
                let exclusive = u.get_mr(1) != 0;
                handle_call(u, |u| s.set_exclusive(u.get_badge(), seat_id, exclusive))
            },

            (protocol::KERNEL_PROTO, protocol::kernel::NOTIFY) => |s: &mut Self, u: &mut UTCB| {
                handle_notify(u, |_u| {
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
