use crate::utils::{InputEvent, InputRequest, get_keymap_as_file, init_xkb_objects};
use calloop::{
    RegistrationToken,
    channel::{Sender, channel},
};
use enumflags2::BitFlags;
use reis::{
    calloop::{EisListenerSource, EisRequestSource, EisRequestSourceEvent},
    eis::{self, device::DeviceType},
    request::{Connection, DeviceCapability, EisRequest},
};
use std::{
    collections::HashMap,
    io,
    os::fd::AsFd,
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, Instant},
};

use std::sync::{Arc, LazyLock, Mutex as StdMutex};
type EisServerSender = Sender<EisServerMsg>;
type InputEventReceiver = Arc<StdMutex<Receiver<InputEvent>>>;

pub static EIS_SERVER: LazyLock<(EisServerSender, InputEventReceiver)> = LazyLock::new(|| {
    let (tx, rx) = start();
    (tx, Arc::new(StdMutex::new(rx)))
});

pub static EIS_SENDER: LazyLock<EisServerSender> = LazyLock::new(|| EIS_SERVER.0.clone());
pub trait SendInputEvent {
    fn send_event(&self, handle: &str, request: InputRequest);
}

impl SendInputEvent for EisServerSender {
    fn send_event(&self, handle: &str, request: InputRequest) {
        let _ = self.send(EisServerMsg::Event(InputEvent {
            session_handle: handle.to_string(),
            request,
        }));
    }
}
pub fn get_input_receiver() -> InputEventReceiver {
    EIS_SERVER.1.clone()
}

#[derive(Debug)]
struct ContextState {
    seat: Option<reis::request::Seat>,
    device_keyboard: Option<reis::request::Device>,
    device_pointer: Option<reis::request::Device>,
    device_pointer_absolute: Option<reis::request::Device>,
    device_touch: Option<reis::request::Device>,
    device_text: Option<reis::request::Device>,
    device_scroll: Option<reis::request::Device>,
    device_button: Option<reis::request::Device>,
    connection: Option<Connection>,
    sequence: u32,
    instant: Instant,
}

impl ContextState {
    fn new() -> Self {
        Self {
            seat: None,
            device_keyboard: None,
            device_pointer: None,
            device_pointer_absolute: None,
            device_touch: None,
            device_text: None,
            device_scroll: None,
            device_button: None,
            connection: None,
            sequence: 0,
            instant: Instant::now(),
        }
    }
    fn handle_input_request(&mut self, request: InputRequest) {
        let current_time = Instant::now();
        let time_stamp = (current_time - self.instant).as_millis() as u64;
        self.sequence += 1;
        match request {
            InputRequest::TouchUp { slot } => {
                if let Some(device) = &self.device_touch
                    && let Some(touch) = device.interface::<reis::eis::Touchscreen>()
                {
                    device.start_emulating(self.sequence);
                    touch.up(slot);
                    device.frame(time_stamp);
                    device.stop_emulating();
                }
            }
            InputRequest::TouchDown { slot, x, y } => {
                if let Some(device) = &self.device_touch
                    && let Some(touch) = device.interface::<reis::eis::Touchscreen>()
                {
                    device.start_emulating(self.sequence);
                    touch.down(slot, x as f32, y as f32);
                    device.frame(time_stamp);
                    device.stop_emulating();
                }
            }
            InputRequest::PointerMotion { dx, dy } => {
                if let Some(device) = &self.device_pointer
                    && let Some(pointer) = device.interface::<reis::eis::Pointer>()
                {
                    device.start_emulating(self.sequence);
                    pointer.motion_relative(dx as f32, dy as f32);
                    device.frame(time_stamp);
                    device.stop_emulating();
                }
            }
            InputRequest::PointerMotionAbsolute { x, y } => {
                if let Some(device) = &self.device_pointer_absolute
                    && let Some(pointer) = device.interface::<reis::eis::PointerAbsolute>()
                {
                    device.start_emulating(self.sequence);
                    pointer.motion_absolute(x as f32, y as f32);
                    device.frame(time_stamp);
                    device.stop_emulating();
                }
            }
            InputRequest::PointerButton { button, state } => {
                if let Some(device) = &self.device_button
                    && let Some(pointer_button) = device.interface::<reis::eis::Button>()
                {
                    device.start_emulating(self.sequence);
                    pointer_button.button(
                        button as u32,
                        if state == 0 {
                            eis::button::ButtonState::Press
                        } else {
                            eis::button::ButtonState::Released
                        },
                    );
                    device.frame(time_stamp);
                    device.stop_emulating();
                }
            }
            InputRequest::KeyboardKeycode { keycode, state } => {
                if let Some(device) = &self.device_keyboard
                    && let Some(keyboard) = device.interface::<reis::eis::Keyboard>()
                {
                    device.start_emulating(self.sequence);
                    keyboard.key(
                        keycode as u32,
                        if state == 0 {
                            eis::keyboard::KeyState::Press
                        } else {
                            eis::keyboard::KeyState::Released
                        },
                    );
                    device.frame(time_stamp);
                    device.stop_emulating();
                }
            }
            InputRequest::PointerAxis { dx, dy, .. } => {
                if let Some(device) = &self.device_scroll
                    && let Some(scroll) = device.interface::<reis::eis::Scroll>()
                {

                    device.start_emulating(self.sequence);
                    scroll.scroll(dx as f32, dy as f32);
                    device.frame(time_stamp);
                    device.stop_emulating();
                }
            }
            InputRequest::PointerAxisDiscrete { axis, steps } => {
                if let Some(device) = &self.device_scroll
                    && let Some(scroll) = device.interface::<reis::eis::Scroll>()
                {
                    device.start_emulating(self.sequence);
                    scroll.scroll_discrete(axis as i32, steps);
                    device.frame(time_stamp);
                    device.stop_emulating();
                }
            }
            _ => {}
        }
        if let Some(connection) = &self.connection {
            let _ = connection.flush();
        }
    }

    fn handle_request(
        &mut self,
        request: &EisRequest,
    ) -> calloop::PostAction {
        match request {
            EisRequest::Disconnect => {
                return calloop::PostAction::Remove;
            }
            EisRequest::Bind(request) => {
                let capabilities = request.capabilities;

                if self.device_keyboard.is_none()
                    && capabilities.contains(DeviceCapability::Keyboard)
                {
                    self.device_keyboard = Some(add_device(
                        "keyboard",
                        BitFlags::from_flag(DeviceCapability::Keyboard),
                        advertise_keyboard_keymap,
                        &request.seat,
                    ));
                }

                if self.device_pointer.is_none() && capabilities.contains(DeviceCapability::Pointer)
                {
                    self.device_pointer = Some(add_device(
                        "pointer",
                        DeviceCapability::Pointer
                            | DeviceCapability::Button
                            | DeviceCapability::Scroll,
                        |_| {},
                        &request.seat,
                    ));
                }

                if self.device_touch.is_none() && capabilities.contains(DeviceCapability::Touch) {
                    self.device_touch = Some(add_device(
                        "touch",
                        BitFlags::from_flag(DeviceCapability::Touch),
                        |_| {},
                        &request.seat,
                    ));
                }

                if self.device_pointer_absolute.is_none()
                    && capabilities.contains(DeviceCapability::PointerAbsolute)
                {
                    self.device_pointer_absolute = Some(add_device(
                        "pointer-abs",
                        DeviceCapability::PointerAbsolute
                            | DeviceCapability::Button
                            | DeviceCapability::Scroll,
                        |_| {},
                        &request.seat,
                    ));
                }
                if self.device_scroll.is_none() && capabilities.contains(DeviceCapability::Scroll) {
                    self.device_scroll = Some(add_device(
                        "scroll",
                        BitFlags::from_flag(DeviceCapability::Scroll),
                        |_| {},
                        &request.seat,
                    ));
                }
                if self.device_scroll.is_none() && capabilities.contains(DeviceCapability::Button) {
                    self.device_button = Some(add_device(
                        "button",
                        BitFlags::from_flag(DeviceCapability::Button),
                        |_| {},
                        &request.seat,
                    ));
                }
                if self.device_text.is_none() && capabilities.contains(DeviceCapability::Text) {
                    self.device_text = Some(add_device(
                        "text",
                        DeviceCapability::Text.into(),
                        |_| {},
                        &request.seat,
                    ));
                }
            }
            _ => {}
        }

        calloop::PostAction::Continue
    }
}

fn advertise_keyboard_keymap(device: &reis::request::Device) {
    let (_, _, state) = init_xkb_objects();
    let (file, size) = get_keymap_as_file(&state);

    if let Some(keyboard) = device.interface::<eis::Keyboard>() {
        keyboard.keymap(eis::keyboard::KeymapType::Xkb, size, file.as_fd());
    } else {
        tracing::error!("Keyboard device is missing its EIS keyboard interface");
    }
}

fn add_device(
    name: &str,
    capabilities: BitFlags<DeviceCapability>,
    before_done_cb: impl for<'a> FnOnce(&'a reis::request::Device),
    seat: &reis::request::Seat,
) -> reis::request::Device {
    let device = seat.add_device(
        Some(name),
        DeviceType::Virtual,
        capabilities,
        before_done_cb,
    );
    device.resumed();
    device
}

struct State {
    handle: calloop::LoopHandle<'static, Self>,
    sender: mpsc::Sender<InputEvent>,
    clients: HashMap<String, RegistrationToken>,
    sessions: HashMap<String, ContextState>,
}

use std::hash::Hash;

use std::sync::atomic::{self, AtomicU32};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Id(u32);

static COUNT: AtomicU32 = AtomicU32::new(0);

impl Id {
    pub fn unique() -> Id {
        Id(COUNT.fetch_add(1, atomic::Ordering::Relaxed))
    }
}

impl State {
    fn handle_new_connection(
        &mut self,
        context: eis::Context,
        session_handle: String,
    ) -> io::Result<calloop::PostAction> {
        tracing::info!(
            "New connection for session {}: {:?}",
            session_handle,
            context
        );

        let source = EisRequestSource::new(context, Id::unique().0);
        let context_state = ContextState::new();
        let session_handle_clone = session_handle.clone();
        self.sessions.insert(session_handle, context_state);
        self.handle
            .insert_source(source, move |event, connected_state, state| {
                Ok(match event {
                    Ok(event)
                        if let Some(context_state) =
                            state.sessions.get_mut(&session_handle_clone) =>
                    {
                        if context_state.connection.is_none() {
                            context_state.connection = Some(connected_state.clone());
                        }
                        Self::handle_request_source_event(
                            context_state,
                            connected_state,
                            event,
                            &state.sender,
                            &session_handle_clone,
                        )
                    }
                    Err(err) => {
                        tracing::error!("Error communicating with client: {err}");
                        calloop::PostAction::Remove
                    }
                    _ => {
                        tracing::error!("context_state not found");
                        calloop::PostAction::Remove
                    }
                })
            })
            .unwrap();

        Ok(calloop::PostAction::Continue)
    }

    fn handle_request_source_event(
        context_state: &mut ContextState,
        connection: &Connection,
        event: EisRequestSourceEvent,
        sender: &mpsc::Sender<InputEvent>,
        session_handle: &str,
    ) -> calloop::PostAction {
        match event {
            EisRequestSourceEvent::Connected => {
                let seat = connection.add_seat(
                    Some("default"),
                    DeviceCapability::Pointer
                        | DeviceCapability::PointerAbsolute
                        | DeviceCapability::Keyboard
                        | DeviceCapability::Touch
                        | DeviceCapability::Scroll
                        | DeviceCapability::Button,
                );

                context_state.seat = Some(seat);
            }
            EisRequestSourceEvent::Request(request) => {
                match &request {
                    EisRequest::PointerMotion(e) => {
                        let _ = sender.send(InputEvent {
                            session_handle: session_handle.to_string(),
                            request: InputRequest::PointerMotion {
                                dx: e.dx as f64,
                                dy: e.dy as f64,
                            },
                        });
                    }
                    EisRequest::PointerMotionAbsolute(e) => {
                        let _ = sender.send(InputEvent {
                            session_handle: session_handle.to_string(),
                            request: InputRequest::PointerMotionAbsolute {
                                x: e.dx_absolute as f64,
                                y: e.dy_absolute as f64,
                            },
                        });
                    }
                    EisRequest::Button(e) => {
                        let _ = sender.send(InputEvent {
                            session_handle: session_handle.to_string(),
                            request: InputRequest::PointerButton {
                                button: e.button as i32,
                                state: e.state as u32,
                            },
                        });
                    }
                    EisRequest::ScrollDelta(e) => {
                        let _ = sender.send(InputEvent {
                            session_handle: session_handle.to_string(),
                            request: InputRequest::PointerAxis {
                                dx: e.dx as f64,
                                dy: e.dy as f64,
                                finish: false,
                            },
                        });
                    }
                    EisRequest::ScrollDiscrete(e) => {
                        if e.discrete_dx != 0 {
                            let _ = sender.send(InputEvent {
                                session_handle: session_handle.to_string(),
                                request: InputRequest::PointerAxisDiscrete {
                                    axis: 1, // Horizontal
                                    steps: e.discrete_dx,
                                },
                            });
                        }
                        if e.discrete_dy != 0 {
                            let _ = sender.send(InputEvent {
                                session_handle: session_handle.to_string(),
                                request: InputRequest::PointerAxisDiscrete {
                                    axis: 0, // Vertical
                                    steps: e.discrete_dy,
                                },
                            });
                        }
                    }
                    EisRequest::KeyboardKey(e) => {
                        let _ = sender.send(InputEvent {
                            session_handle: session_handle.to_string(),
                            request: InputRequest::KeyboardKeycode {
                                keycode: e.key as i32,
                                state: e.state as u32,
                            },
                        });
                    }
                    EisRequest::TouchDown(e) => {
                        let _ = sender.send(InputEvent {
                            session_handle: session_handle.to_string(),
                            request: InputRequest::TouchDown {
                                slot: e.touch_id,
                                x: e.x as f64,
                                y: e.y as f64,
                            },
                        });
                    }
                    EisRequest::TouchMotion(e) => {
                        let _ = sender.send(InputEvent {
                            session_handle: session_handle.to_string(),
                            request: InputRequest::TouchMotion {
                                slot: e.touch_id,
                                x: e.x as f64,
                                y: e.y as f64,
                            },
                        });
                    }
                    EisRequest::TouchUp(e) => {
                        let _ = sender.send(InputEvent {
                            session_handle: session_handle.to_string(),
                            request: InputRequest::TouchUp { slot: e.touch_id },
                        });
                    }
                    _ => {}
                }

                let res = context_state.handle_request(&request);
                if res != calloop::PostAction::Continue {
                    return res;
                }
            }
        }

        let _ = connection.flush();

        calloop::PostAction::Continue
    }
}

#[allow(clippy::enum_variant_names)]
pub enum EisServerMsg {
    NewListener(eis::Listener, String),
    RemoveListener(String),
    StopContext(String),
    ActiveContext(String),
    RemoveContext(String),
    Event(InputEvent),
}

pub fn start() -> (Sender<EisServerMsg>, Receiver<InputEvent>) {
    let (tx, msg_channel) = channel();
    let (input_tx, input_rx) = mpsc::channel();

    thread::spawn(move || {
        let mut event_loop = calloop::EventLoop::<State>::try_new().unwrap();
        let handle = event_loop.handle();
        let mut state = State {
            handle: handle.clone(),
            sender: input_tx,
            clients: HashMap::new(),
            sessions: HashMap::new(),
        };

        let _ = handle.insert_source(msg_channel, |event, _, state| {
            if let calloop::channel::Event::Msg(msg) = event {
                match msg {
                    EisServerMsg::NewListener(listener, session_handle) => {
                        let listener_source = EisListenerSource::new(listener);
                        let session_handle_2 = session_handle.clone();
                        let token = state
                            .handle
                            .insert_source(
                                listener_source,
                                move |context, (), state: &mut State| {
                                    state.handle_new_connection(context, session_handle.clone())
                                },
                            )
                            .unwrap();
                        state.clients.insert(session_handle_2, token);
                    }
                    EisServerMsg::RemoveListener(session) => {
                        state.sessions.remove(&session);
                        let Some(token) = state.clients.remove(&session) else {
                            return;
                        };
                        state.handle.remove(token);
                    }
                    EisServerMsg::StopContext(session) => {
                        let Some(token) = state.clients.get(&session) else {
                            return;
                        };
                        let _ = state.handle.disable(token);
                    }
                    EisServerMsg::ActiveContext(session) => {
                        let Some(token) = state.clients.get(&session) else {
                            return;
                        };
                        let _ = state.handle.enable(token);
                    }
                    EisServerMsg::RemoveContext(session) => {
                        let Some(token) = state.clients.remove(&session) else {
                            return;
                        };
                        state.handle.remove(token);
                    }
                    EisServerMsg::Event(InputEvent {
                        session_handle,
                        request,
                    }) => {
                        let Some(session) = state.sessions.get_mut(&session_handle) else {
                            return;
                        };
                        session.handle_input_request(request);
                    }
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
