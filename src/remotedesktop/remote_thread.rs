use wayland_client::Connection;
use wayland_client::globals::registry_queue_init;
use wayland_client::protocol::wl_keyboard;
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1;
use wayland_protocols_wlr::virtual_pointer::v1::client::zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1;

use crate::remotedesktop::dispatch::init_xkb_objects;

use super::dispatch::get_keymap_as_file;
use super::state::AppData;
use super::state::KeyPointerError;
use std::sync::Mutex;
use std::sync::atomic::Ordering;

use std::os::fd::AsFd;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{self, Receiver, Sender};

use calloop::EventLoop;
use calloop_wayland_source::WaylandSource;

#[derive(Debug)]
pub enum InputRequest {
    PointerMotion { dx: f64, dy: f64 },
    PointerMotionAbsolute { x: f64, y: f64 },
    PointerButton { button: i32, state: u32 },
    PointerAxis { dx: f64, dy: f64 },
    PointerAxisDiscrate { axis: u32, steps: i32 },
    KeyboardKeycode { keycode: i32, state: u32 },
    KeyboardKeysym { keysym: i32, state: u32 },
    Exit,
}

#[derive(Debug)]
pub struct RemoteControl {
    pub sender: Sender<InputRequest>,
}

impl RemoteControl {
    pub fn init(output_width: u32, output_height: u32) -> Self {
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = remote_loop(receiver, output_width, output_height);
        });
        Self { sender }
    }

    pub fn stop(&self) {
        let _ = self.sender.send(InputRequest::Exit);
    }
}

pub fn remote_loop(
    receiver: Receiver<InputRequest>,
    output_width: u32,
    output_height: u32,
) -> Result<(), KeyPointerError> {
    // Create a Wayland connection by connecting to the server through the
    // environment-provided configuration.
    let conn = Connection::connect_to_env().map_err(|_| {
        KeyPointerError::InitFailedConnection("Cannot create connection".to_string())
    })?;

    // Retrieve the WlDisplay Wayland object from the connection. This object is
    // the starting point of any Wayland program, from which all other objects will
    // be created.
    let display = conn.display();

    let (globals, mut event_queue) = registry_queue_init::<AppData>(&conn)?; // We just need the

    let qh = event_queue.handle();
    let seat = globals.bind::<WlSeat, _, _>(&qh, 7..=9, ())?;
    let virtual_keyboard_manager =
        globals.bind::<ZwpVirtualKeyboardManagerV1, _, _>(&qh, 1..=1, ())?;

    let virtual_keyboard = virtual_keyboard_manager.create_virtual_keyboard(&seat, &qh, ());
    let (xkb_context, xkb_keymap, xkb_state) = init_xkb_objects();
    let (file, size) = get_keymap_as_file(&xkb_state);
    virtual_keyboard.keymap(wl_keyboard::KeymapFormat::XkbV1.into(), file.as_fd(), size);

    let virtual_pointer_manager =
        globals.bind::<ZwlrVirtualPointerManagerV1, _, _>(&qh, 1..=2, ())?;
    let pointer = virtual_pointer_manager.create_virtual_pointer(Some(&seat), &qh, ());
    // Create an event queue for our event processing
    // An get its handle to associated new objects to it

    // Create a wl_registry object by sending the wl_display.get_registry request
    // This method takes two arguments: a handle to the queue the newly created
    // wl_registry will be assigned to, and the user-data that should be associated
    // with this registry (here it is () as we don't need user-data).
    let _registry = display.get_registry(&qh, ());
    let keyboard = seat.get_keyboard(&qh, ());
    let mut data = AppData::new(
        virtual_keyboard,
        pointer,
        xkb_context,
        xkb_keymap,
        xkb_state,
        output_width,
        output_height,
    );
    let _ = event_queue.roundtrip(&mut data);
    keyboard.release();

    let mut event_loop: EventLoop<AppData> =
        EventLoop::try_new().expect("Failed to initialize the event loop");

    WaylandSource::new(conn, event_queue)
        .insert(event_loop.handle())
        .expect("Failed to init wayland source");

    let to_exit = Arc::new(AtomicBool::new(false));

    let events: Arc<Mutex<Vec<InputRequest>>> = Arc::new(Mutex::new(Vec::new()));

    let to_exit2 = to_exit.clone();
    let to_exit3 = to_exit.clone();
    let events_2 = events.clone();
    let thread = std::thread::spawn(move || {
        let to_exit = to_exit2;
        let events = events_2;

        for message in receiver.iter() {
            if to_exit.load(Ordering::Relaxed) {
                break;
            }
            let mut events_local = events.lock().unwrap();
            events_local.push(message);
        }
        to_exit.store(true, Ordering::Relaxed);
    });

    // At this point everything is ready, and we just need to wait to receive the events
    // from the wl_registry, our callback will print the advertized globals.

    let signal = event_loop.get_signal();
    event_loop
        .run(
            std::time::Duration::from_millis(20),
            &mut data,
            move |data| {
                if to_exit3.load(Ordering::Relaxed) {
                    signal.stop();
                    return;
                }
                let mut local_events = events.lock().expect(
                    "This events only used in this callback, so it should always can be unlocked",
                );
                let mut swapped_events = vec![];
                std::mem::swap(&mut *local_events, &mut swapped_events);
                drop(local_events);
                for message in swapped_events {
                    match message {
                        InputRequest::PointerMotion { dx, dy } => {
                            data.notify_pointer_motion(dx, dy)
                        }
                        InputRequest::PointerMotionAbsolute { x, y } => {
                            data.notify_pointer_motion_absolute(x, y)
                        }
                        InputRequest::PointerButton { button, state } => {
                            data.notify_pointer_button(button, state)
                        }
                        InputRequest::PointerAxis { dx, dy } => data.notify_pointer_axis(dx, dy),
                        InputRequest::PointerAxisDiscrate { axis, steps } => {
                            data.notify_pointer_axis_discrete(axis, steps)
                        }
                        InputRequest::KeyboardKeycode { keycode, state } => {
                            data.notify_keyboard_keycode(keycode, state)
                        }
                        InputRequest::KeyboardKeysym { keysym, state } => {
                            data.notify_keyboard_keysym(keysym, state)
                        }
                        InputRequest::Exit => {
                            signal.stop();
                            break;
                        }
                    }
                }
            },
        )
        .expect("Error during event loop");

    to_exit.store(true, Ordering::Relaxed);
    let _ = thread.join();

    Ok(())
}
