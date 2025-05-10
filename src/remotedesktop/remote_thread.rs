use wayland_client::Connection;
use wayland_client::globals::registry_queue_init;
use wayland_client::protocol::wl_keyboard;
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1;
use wayland_protocols_wlr::virtual_pointer::v1::client::zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1;

use super::dispatch::get_keymap_as_file;
use super::state::AppData;
use super::state::KeyPointerError;

use std::os::fd::AsFd;
use std::sync::mpsc::{self, Receiver, Sender};

#[derive(Debug)]
pub enum KeyOrPointerRequest {
    PointerMotion {
        dx: f64,
        dy: f64,
    },
    PointerMotionAbsolute {
        x: f64,
        y: f64,
        x_extent: u32,
        y_extent: u32,
    },
    PointerButton {
        button: i32,
        state: u32,
    },
    PointerAxis {
        dx: f64,
        dy: f64,
    },
    PointerAxisDiscrate {
        axis: u32,
        steps: i32,
    },
    KeyboardKeycode {
        keycode: i32,
        state: u32,
    },
    KeyboardKeysym {
        keysym: i32,
        state: u32,
    },
    Exit,
}

#[derive(Debug)]
pub struct RemoteControl {
    pub sender: Sender<KeyOrPointerRequest>,
}

impl RemoteControl {
    pub fn init() -> Self {
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = remote_loop(receiver);
        });
        Self { sender }
    }

    pub fn stop(&self) {
        let _ = self.sender.send(KeyOrPointerRequest::Exit);
    }
}

pub fn remote_loop(receiver: Receiver<KeyOrPointerRequest>) -> Result<(), KeyPointerError> {
    // Create a Wayland connection by connecting to the server through the
    // environment-provided configuration.
    let conn = Connection::connect_to_env()
        .map_err(|_| KeyPointerError::ConnectionError("Cannot create connection".to_string()))?;

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
    let (file, size) = get_keymap_as_file();
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

    // At this point everything is ready, and we just need to wait to receive the events
    // from the wl_registry, our callback will print the advertized globals.
    let mut data = AppData::new(virtual_keyboard, pointer);

    while let Ok(message) = receiver.recv() {
        match message {
            KeyOrPointerRequest::PointerMotion { dx, dy } => data.notify_pointer_motion(dx, dy),
            KeyOrPointerRequest::PointerMotionAbsolute {
                x,
                y,
                x_extent,
                y_extent,
            } => data.notify_pointer_motion_absolute(x, y, x_extent, y_extent),
            KeyOrPointerRequest::PointerButton { button, state } => {
                data.notify_pointer_button(button, state)
            }
            KeyOrPointerRequest::PointerAxis { dx, dy } => data.notify_pointer_axis(dx, dy),
            KeyOrPointerRequest::PointerAxisDiscrate { axis, steps } => {
                data.notify_pointer_axis_discrete(axis, steps)
            }
            KeyOrPointerRequest::KeyboardKeycode { keycode, state } => {
                data.notify_keyboard_keycode(keycode, state)
            }
            KeyOrPointerRequest::KeyboardKeysym { keysym, state } => {
                data.notify_keyboard_keysym(keysym, state)
            }
            KeyOrPointerRequest::Exit => break,
        }
        event_queue.roundtrip(&mut data).ok();
    }

    Ok(())
}
