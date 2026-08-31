use calloop::{
    RegistrationToken,
    channel::{Sender, channel},
    generic::Generic,
};
use enumflags2::BitFlags;
use reis::{PendingRequestResult, ei};
use std::sync::LazyLock;
use std::{
    collections::HashMap,
    io,
    os::fd::AsFd,
    sync::mpsc::{self, Receiver},
    thread,
    time::Duration,
};
pub enum EiClientMsg {
    NewListener(ei::Context, String),
    StopListener(String),
    ActiveListener(String),
    RemoveListener(String),
}

static INTERFACES: LazyLock<HashMap<&'static str, u32>> = LazyLock::new(|| {
    let mut m = HashMap::new();
    m.insert("ei_callback", 1);
    m.insert("ei_connection", 1);
    m.insert("ei_seat", 1);
    m.insert("ei_device", 1);
    m.insert("ei_pingpong", 1);
    m.insert("ei_keyboard", 1);
    m.insert("ei_button", 1);
    m.insert("ei_pointer", 1);
    m
});

#[derive(Debug, Default)]
struct SeatData {
    name: Option<String>,
    capabilities: HashMap<String, u64>,
}

#[derive(Debug, Default)]
struct DeviceData {
    name: Option<String>,
    device_type: Option<ei::device::DeviceType>,
    interfaces: HashMap<String, reis::Object>,
}

impl DeviceData {
    fn interface<T: reis::Interface>(&self) -> Option<T> {
        self.interfaces.get(T::NAME)?.clone().downcast()
    }
}
#[derive(Debug)]
pub struct State {
    handle: calloop::LoopHandle<'static, Self>,
    sender: mpsc::Sender<InputEvent>,
    clients: HashMap<String, RegistrationToken>,
    // XXX best way to handle data associated with object?
    seats: HashMap<ei::Seat, SeatData>,
    // XXX association with seat?
    devices: HashMap<ei::Device, DeviceData>,
    sequence: u32,
    last_serial: u32,
}

impl State {
    #![allow(clippy::unnecessary_wraps)]
    fn handle_listener_readable(
        &mut self,
        context: &mut ei::Context,
    ) -> io::Result<calloop::PostAction> {
        if context.read().is_err() {
            return Ok(calloop::PostAction::Remove);
        }

        while let Some(result) = context.pending_event() {
            let request = match result {
                PendingRequestResult::Request(request) => request,
                PendingRequestResult::ParseError(_msg) => {
                    todo!()
                }
                PendingRequestResult::InvalidObject(_object_id) => {
                    // TODO
                    continue;
                }
            };
            match request {
                ei::Event::Handshake(handshake, request) => match request {
                    ei::handshake::Event::HandshakeVersion { version: _ } => {
                        handshake.handshake_version(1);
                        handshake.name("xdpl-ei");
                        handshake.context_type(ei::handshake::ContextType::Sender);
                        for (interface, version) in INTERFACES.iter() {
                            handshake.interface_version(interface, *version);
                        }
                        handshake.finish();
                    }
                    ei::handshake::Event::Connection {
                        connection: _,
                        serial,
                    } => {
                        self.last_serial = serial;
                    }
                    _ => {}
                },
                ei::Event::Connection(_connection, request) => match request {
                    ei::connection::Event::Seat { seat } => {
                        self.seats.insert(seat, SeatData::default());
                    }
                    ei::connection::Event::Ping { ping } => {
                        ping.done(0);
                    }
                    _ => {}
                },
                ei::Event::Seat(seat, request) => {
                    let data = self.seats.get_mut(&seat).unwrap();
                    match request {
                        ei::seat::Event::Name { name } => {
                            data.name = Some(name);
                        }
                        ei::seat::Event::Capability { mask, interface } => {
                            data.capabilities.insert(interface, mask);
                        }
                        ei::seat::Event::Done => {
                            if let Some(mask) = data.capabilities.get("ei_text") {
                                seat.bind(*mask);
                            } else {
                                tracing::warn!(
                                    "Server does not advertise the ei_text capability \
                                     (requires libei 1.6.0 or newer)."
                                );
                            }
                        }
                        ei::seat::Event::Device { device } => {
                            self.devices.insert(device, DeviceData::default());
                        }
                        _ => {}
                    }
                }
                ei::Event::Device(device, request) => {
                    let data = self.devices.get_mut(&device).unwrap();
                    match request {
                        ei::device::Event::Name { name } => {
                            data.name = Some(name);
                        }
                        ei::device::Event::DeviceType { device_type } => {
                            data.device_type = Some(device_type);
                        }
                        ei::device::Event::Interface { object } => {
                            data.interfaces
                                .insert(object.interface().to_owned(), object);
                        }
                        ei::device::Event::Done => {
                            if let Some(text) = data.interface::<ei::Text>() {
                                // ei_text takes a UTF-8 string directly, so there is no
                                // need to reverse-map characters to keycodes through a keymap.
                                device.start_emulating(self.sequence, self.last_serial);
                                self.sequence += 1;
                                text.utf8("Hello world!");
                                device.frame(self.last_serial, 1); // XXX time
                                device.stop_emulating(self.last_serial);
                                //self.running = false;
                            }
                        }
                        ei::device::Event::Resumed { serial } => {
                            self.last_serial = serial;
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }

        let _ = context.flush();

        Ok(calloop::PostAction::Continue)
    }
}

pub enum InputEvent {}

pub fn start() -> (Sender<EiClientMsg>, Receiver<InputEvent>) {
    let (tx, msg_channel) = channel();
    let (input_tx, input_rx) = mpsc::channel();
    thread::spawn(move || {
        let mut event_loop = calloop::EventLoop::<State>::try_new().unwrap();
        let handle = event_loop.handle();
        let mut state = State {
            handle: handle.clone(),
            sender: input_tx,
            clients: HashMap::new(),
            seats: HashMap::new(),
            devices: HashMap::new(),
            last_serial: u32::MAX,
            sequence: 0,
        };

        let _ = handle.insert_source(msg_channel, |event, _, state| {
            let calloop::channel::Event::Msg(msg) = event else {
                return;
            };

            match msg {
                EiClientMsg::NewListener(context, session_handle) => {
                    let session_handle_2 = session_handle.clone();
                    let _handlshake = context.handshake();
                    let _ = context.flush();
                    let context_source =
                        Generic::new(context, calloop::Interest::READ, calloop::Mode::Level);
                    let token = state
                        .handle
                        .insert_source(context_source, move |_event, context, state: &mut State| {
                            state.handle_listener_readable(unsafe { context.get_mut() })
                        })
                        .unwrap();
                    state.clients.insert(session_handle_2, token);
                }
                EiClientMsg::StopListener(session) => {
                    let Some(token) = state.clients.get(&session) else {
                        return;
                    };
                    let _ = state.handle.disable(token);
                }
                EiClientMsg::ActiveListener(session) => {
                    let Some(token) = state.clients.get(&session) else {
                        return;
                    };
                    let _ = state.handle.enable(token);
                }
                EiClientMsg::RemoveListener(session) => {
                    let Some(token) = state.clients.remove(&session) else {
                        return;
                    };
                    state.handle.remove(token);
                }
            }
        });

        loop {
            event_loop
                .dispatch(Duration::from_millis(100), &mut state)
                .unwrap();
        }
    });
    (tx, input_rx)
}
