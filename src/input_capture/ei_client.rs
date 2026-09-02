use calloop::{
    RegistrationToken,
    channel::{Sender, channel},
    generic::Generic,
};
use reis::{PendingRequestResult, ei};
use std::sync::LazyLock;
use std::{
    collections::HashMap,
    io,
    thread,
    time::Duration,
};

use crate::utils::{InputEvent, InputRequest};

#[derive(Debug, Clone)]
pub enum EiClientMsg {
    NewContext(ei::Context, String),
    StopContext(String),
    ActiveContext(String),
    RemoveContext(String),
    Event(InputEvent),
}

const INTERFACES_LIST: &[&'static str] = &[
    "ei_callback",
    "ei_connection",
    "ei_seat",
    "ei_device",
    "ei_pingpong",
    "ei_keyboard",
    "ei_button",
    "ei_touch",
    "ei_pointer",
    "ei_pointer_absolute",
    "ei_scroll"
];

static INTERFACES: LazyLock<HashMap<&'static str, u32>> = LazyLock::new(|| {
    let mut m = HashMap::new();
    for interface in INTERFACES_LIST {
        m.insert(*interface, 1);
    }
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

// TODO: the keyboard , seats should be related to the context
#[derive(Debug)]
pub struct State {
    handle: calloop::LoopHandle<'static, Self>,
    clients: HashMap<String, RegistrationToken>,
    // XXX best way to handle data associated with object?
    sessions: HashMap<String, SessionState>,
}

#[derive(Debug, Default)]
pub struct SessionState {
    // XXX best way to handle data associated with object?
    seats: HashMap<ei::Seat, SeatData>,
    // XXX association with seat?
    devices: HashMap<ei::Device, DeviceData>,
    last_serial: u32,

    keyboard: Option<ei::Keyboard>,
    pointer: Option<ei::Pointer>,
    pointer_abs: Option<ei::PointerAbsolute>,
    button: Option<ei::Button>,
    touch: Option<ei::Touchscreen>,
    scroll: Option<ei::Scroll>
}

impl SessionState {
    fn new() -> Self {
        Self::default()
    }
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
                    continue;
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
                            for interface in INTERFACES_LIST {
                                if let Some(mask) = data.capabilities.get(*interface) {
                                    seat.bind(*mask);
                                }
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
                            if self.keyboard.is_none()
                                && let Some(keyboard) = data.interface::<ei::Keyboard>()
                            {
                                self.keyboard = Some(keyboard);
                            }
                            if self.pointer.is_none()
                                && let Some(pointer) = data.interface::<ei::Pointer>()
                            {
                                self.pointer = Some(pointer);
                            }
                            if self.pointer_abs.is_none()
                                && let Some(pointer_abs) = data.interface::<ei::PointerAbsolute>()
                            {
                                self.pointer_abs = Some(pointer_abs);
                            }
                            if self.button.is_none()
                                && let Some(button) = data.interface::<ei::Button>()
                            {
                                self.button = Some(button);
                            }
                            if self.touch.is_none()
                                && let Some(touch) = data.interface::<ei::Touchscreen>()
                            {
                                self.touch = Some(touch);
                            }
                            if self.scroll.is_none()
                                && let Some(scroll) = data.interface::<ei::Scroll>()
                            {
                                self.scroll = Some(scroll);
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

    fn handle_request(&self, request: InputRequest) {
        match request {
            InputRequest::TouchUp { slot } => {
                if let Some(touch) = &self.touch {
                    touch.up(slot);
                }
            }
            InputRequest::TouchDown { slot, x, y } => {
                if let Some(touch) = &self.touch {
                    touch.down(slot, x as f32, y as f32);
                }
            }
            InputRequest::PointerMotion { dx, dy } => {
                if let Some(pointer) = &self.pointer {
                    pointer.motion_relative(dx as f32, dy as f32);
                }
            }
            InputRequest::PointerMotionAbsolute { x, y } => {
                if let Some(pointer) = &self.pointer_abs {
                    pointer.motion_absolute(x as f32, y as f32);
                }
            }
            InputRequest::PointerButton { button, state } => {
                if let Some(pointer_button) = &self.button {
                    pointer_button.button(
                        button as u32,
                        if state == 0 {
                            ei::button::ButtonState::Press
                        } else {
                            ei::button::ButtonState::Released
                        },
                    );
                }
            }
            InputRequest::KeyboardKeycode { keycode, state } => {
                if let Some(keyboard) = &self.keyboard {
                    keyboard.key(
                        keycode as u32,
                        if state == 0 {
                            ei::keyboard::KeyState::Press
                        } else {
                            ei::keyboard::KeyState::Released
                        },
                    );
                }
            }
            InputRequest::PointerAxis { dx, dy, .. } => {
                if let Some(scroll) = &self.scroll {
                    scroll.scroll(dx as f32, dy as f32);
                }
            }
            InputRequest::PointerAxisDiscrete { axis, steps } => {
                if let Some(scroll) = &self.scroll {
                    scroll.scroll_discrete(axis as i32, steps);
                }
            }
            _ => {}
        }
    }
}

impl State {
    fn handle_listener_readable(
        &mut self,
        session_handle: String,
        context: &mut ei::Context,
    ) -> io::Result<calloop::PostAction> {
        let mut session = SessionState::new();
        let re = session.handle_listener_readable(context);
        self.sessions.insert(session_handle, session);
        re
    }
}

pub fn start() -> Sender<EiClientMsg> {
    let (tx, msg_channel) = channel();
    thread::spawn(move || {
        let mut event_loop = calloop::EventLoop::<State>::try_new().unwrap();
        let handle = event_loop.handle();
        let mut state = State {
            handle: handle.clone(),
            clients: HashMap::new(),
            sessions: HashMap::new(),
        };

        let _ = handle.insert_source(msg_channel, |event, _, state| {
            let calloop::channel::Event::Msg(msg) = event else {
                return;
            };

            match msg {
                EiClientMsg::NewContext(context, session_handle) => {
                    let session_handle_2 = session_handle.clone();
                    let _handlshake = context.handshake();
                    let _ = context.flush();
                    let context_source =
                        Generic::new(context, calloop::Interest::READ, calloop::Mode::Level);
                    let token = state
                        .handle
                        .insert_source(context_source, move |_event, context, state: &mut State| {
                            let session_handle_3 = session_handle.clone();
                            state.handle_listener_readable(session_handle_3, unsafe {
                                context.get_mut()
                            })
                        })
                        .unwrap();
                    state.clients.insert(session_handle_2, token);
                }
                EiClientMsg::StopContext(session) => {
                    let Some(token) = state.clients.get(&session) else {
                        return;
                    };
                    let _ = state.handle.disable(token);
                }
                EiClientMsg::ActiveContext(session) => {
                    let Some(token) = state.clients.get(&session) else {
                        return;
                    };
                    let _ = state.handle.enable(token);
                }
                EiClientMsg::RemoveContext(session) => {
                    let Some(token) = state.clients.remove(&session) else {
                        return;
                    };
                    state.handle.remove(token);
                }
                EiClientMsg::Event(InputEvent {
                    session_handle,
                    request,
                }) => {
                    let Some(session) = state.sessions.get(&session_handle) else {
                        return;
                    };
                    session.handle_request(request);
                }
            }
        });

        loop {
            event_loop
                .dispatch(Duration::from_millis(100), &mut state)
                .unwrap();
        }
    });
    tx
}
