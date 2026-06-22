use std::{
    collections::HashMap,
    os::fd::{AsFd, AsRawFd, OwnedFd},
};

use calloop_wayland_source::WaylandSource;
use sctk::registry::{ProvidesRegistryState, RegistryState};
use wayland_protocols::ext::data_control::v1::client::{
    ext_data_control_device_v1::{self, ExtDataControlDeviceV1},
    ext_data_control_manager_v1::ExtDataControlManagerV1,
    ext_data_control_offer_v1::{self, ExtDataControlOfferV1},
    ext_data_control_source_v1,
};

use calloop::{
    EventLoop,
    channel::{self, Channel, Sender},
};
use tokio::sync::oneshot::Sender as OneSender;
use wayland_client::{
    Connection, Dispatch, QueueHandle, delegate_noop, event_created_child,
    globals::registry_queue_init, protocol::wl_seat::WlSeat,
};

use os_pipe::{PipeReader, pipe};

pub struct ClipboardThread {
    pub sender: Sender<ClipboardRequest>,
}

impl ClipboardThread {
    pub fn new() -> Self {
        let (sender, receiver) = channel::channel();
        std::thread::spawn(move || {
            let _ = clipboard_loop(receiver);
        });
        Self { sender }
    }

    pub fn stop(&self) {
        let _ = self.sender.send(ClipboardRequest::Stop);
    }
}

pub struct ClipboardWl {
    registry_state: RegistryState,
    seat: WlSeat,
    data_manager: ExtDataControlManagerV1,
    device: ExtDataControlDeviceV1,
    qh: QueueHandle<Self>,
    current_selection: Option<ExtDataControlOfferV1>,
    primary_selection: Option<ExtDataControlOfferV1>,
    piperead: Option<PipeReader>,
    mime_types: Vec<String>,
    write_data: HashMap<i32, ClipboardData>,
}

struct ClipboardData {
    sender: OneSender<(OwnedFd, String)>,
    #[allow(unused)]
    serial: u32,
}

impl Dispatch<ext_data_control_device_v1::ExtDataControlDeviceV1, ()> for ClipboardWl {
    fn event(
        state: &mut Self,
        _proxy: &ext_data_control_device_v1::ExtDataControlDeviceV1,
        event: <ext_data_control_device_v1::ExtDataControlDeviceV1 as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &wayland_client::Connection,
        _qhandle: &wayland_client::QueueHandle<Self>,
    ) {
        match event {
            ext_data_control_device_v1::Event::DataOffer { .. } => {}
            ext_data_control_device_v1::Event::Finished => {
                state.reset_offer();
                state.reset_data_device();
            }
            ext_data_control_device_v1::Event::Selection { id } => {
                if let Some(offer) = id {
                    state.current_selection = Some(offer);
                }
            }
            ext_data_control_device_v1::Event::PrimarySelection { id } => {
                // NOTE: we need to manager its lifetime
                state.reset_primary_offer(id);
            }
            _ => unreachable!(),
        }
    }
    event_created_child!(ClipboardWl, ext_data_control_device_v1::ExtDataControlDeviceV1, [
        ext_data_control_device_v1::EVT_DATA_OFFER_OPCODE => (ext_data_control_offer_v1::ExtDataControlOfferV1, ())
    ]);
}

impl Dispatch<ext_data_control_source_v1::ExtDataControlSourceV1, ()> for ClipboardWl {
    fn event(
        state: &mut Self,
        _proxy: &ext_data_control_source_v1::ExtDataControlSourceV1,
        event: <ext_data_control_source_v1::ExtDataControlSourceV1 as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &wayland_client::Connection,
        _qhandle: &wayland_client::QueueHandle<Self>,
    ) {
        match event {
            ext_data_control_source_v1::Event::Send { mime_type, fd } => {
                let raw_fd = fd.as_raw_fd();
                let Some(data) = state.write_data.remove(&raw_fd) else {
                    return;
                };
                let _ = data.sender.send((fd, mime_type));
            }
            ext_data_control_source_v1::Event::Cancelled => {
                // TODO: maybe should one request to one offer?
                state.write_data.clear();
            }
            _ => unreachable!(),
        }
    }
}

impl Dispatch<ext_data_control_offer_v1::ExtDataControlOfferV1, ()> for ClipboardWl {
    fn event(
        _state: &mut Self,
        _proxy: &ext_data_control_offer_v1::ExtDataControlOfferV1,
        _event: <ext_data_control_offer_v1::ExtDataControlOfferV1 as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &wayland_client::Connection,
        _qhandle: &wayland_client::QueueHandle<Self>,
    ) {
    }
}

impl ProvidesRegistryState for ClipboardWl {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    sctk::registry_handlers![];
}

delegate_noop!(ClipboardWl: ignore WlSeat);
delegate_noop!(ClipboardWl: ignore ExtDataControlManagerV1);
sctk::delegate_registry!(ClipboardWl);

pub enum ClipboardRequest {
    SetSelection {
        mime_types: Vec<String>,
    },
    Write {
        sender: OneSender<(OwnedFd, String)>,
        serial: u32,
    },

    Read {
        sender: OneSender<OwnedFd>,
        mime_type: String,
    },
    Stop,
}

impl ClipboardWl {
    fn reset_data_device(&mut self) {
        self.device.destroy();
        self.device = self.data_manager.get_data_device(&self.seat, &self.qh, ());
    }
    fn reset_offer(&mut self) {
        self.current_selection.take().map(|offer| offer.destroy());
    }
    fn reset_primary_offer(&mut self, id: Option<ExtDataControlOfferV1>) {
        if let Some(id) = id {
            self.primary_selection.take().map(|offer| offer.destroy());
            self.primary_selection = Some(id);
        }
    }
    pub fn select_selection(&mut self, mime_types: Vec<String>) -> anyhow::Result<()> {
        self.mime_types = mime_types.clone();
        self.reset_data_device();
        let source = self.data_manager.create_data_source(&self.qh, ());

        for mime_type in mime_types {
            source.offer(mime_type);
        }
        self.device.set_selection(Some(&source));
        Ok(())
    }
}
pub fn clipboard_loop(receiver: Channel<ClipboardRequest>) -> anyhow::Result<()> {
    let connection = Connection::connect_to_env()?;
    let (globals, event_queue) = registry_queue_init::<ClipboardWl>(&connection)?;
    let qh = event_queue.handle();
    let seat = globals.bind::<WlSeat, _, _>(&qh, 1..=1, ())?;
    let data_manager = globals.bind::<ExtDataControlManagerV1, _, _>(&qh, 1..=1, ())?;
    let device = data_manager.get_data_device(&seat, &qh, ());
    let mut clipbard = ClipboardWl {
        registry_state: RegistryState::new(&globals),
        seat,
        data_manager,
        device,
        qh,
        current_selection: None,
        primary_selection: None,
        piperead: None,
        mime_types: Vec::new(),
        write_data: HashMap::new(),
    };

    let mut event_loop: EventLoop<ClipboardWl> =
        EventLoop::try_new().expect("Failed to initialize the event loop");

    let signal = event_loop.get_signal();
    WaylandSource::new(connection, event_queue)
        .insert(event_loop.handle())
        .expect("Failed to init wayland source");
    event_loop
        .handle()
        .insert_source(receiver, move |event, _, app_state| {
            let channel::Event::Msg(message) = event else {
                return;
            };

            match message {
                ClipboardRequest::SetSelection { mime_types } => {
                    let _ = app_state.select_selection(mime_types);
                }
                ClipboardRequest::Read { sender, mime_type } => {
                    let Some(offer) = &app_state.current_selection else {
                        return;
                    };
                    let (read, write) = pipe().unwrap();

                    offer.receive(mime_type, write.as_fd());
                    let _ = sender.send(read.into());
                }
                ClipboardRequest::Write { sender, serial } => {
                    let Some(offer) = &app_state.current_selection else {
                        return;
                    };

                    let (read, write) = pipe().unwrap();
                    for mime_type in &app_state.mime_types {
                        offer.receive(mime_type.clone(), write.as_fd());
                    }
                    let raw_fd = write.as_raw_fd();
                    app_state.piperead = Some(read);
                    app_state
                        .write_data
                        .insert(raw_fd, ClipboardData { sender, serial });
                }
                ClipboardRequest::Stop => {
                    signal.stop();
                }
            }
        })
        .expect("Error during event loop");
    event_loop
        .run(
            std::time::Duration::from_millis(20),
            &mut clipbard,
            |_data| {},
        )
        .expect("Error during event loop");
    Ok(())
}
