use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1;

use wayland_client::{
    DispatchError,
    globals::{BindError, GlobalError},
    protocol::wl_pointer,
};
use wayland_protocols_wlr::virtual_pointer::v1::client::zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1;

use thiserror::Error;
// This struct represents the state of our app. This simple app does not
// need any state, by this type still supports the `Dispatch` implementations.
#[derive(Debug)]
pub struct AppData {
    pub(crate) virtual_keyboard: ZwpVirtualKeyboardV1,

    pub(crate) virtual_pointer: ZwlrVirtualPointerV1,
}

impl AppData {
    pub fn new(
        virtual_keyboard: ZwpVirtualKeyboardV1,
        virtual_pointer: ZwlrVirtualPointerV1,
    ) -> Self {
        Self {
            virtual_keyboard,
            virtual_pointer,
        }
    }
}

impl Drop for AppData {
    fn drop(&mut self) {
        self.virtual_pointer.destroy();
        self.virtual_keyboard.destroy();
    }
}

#[derive(Error, Debug)]
pub enum KeyPointerError {
    #[error("Connection create Error")]
    ConnectionError(String),
    #[error("Error during queue")]
    QueueError(#[from] DispatchError),
    #[error("GlobalError")]
    GlobalError(#[from] GlobalError),
    #[error("BindError")]
    BindError(#[from] BindError),
}

impl AppData {
    pub fn notify_pointer_motion(&self, dx: f64, dy: f64) {
        self.virtual_pointer.motion(10, dx, dy);
    }

    pub fn notify_pointer_motion_absolute(&self, x: f64, y: f64, x_extent: u32, y_extent: u32) {
        self.virtual_pointer
            .motion_absolute(10, x as u32, y as u32, x_extent, y_extent);
    }

    pub fn notify_pointer_button(&self, button: i32, state: u32) {
        self.virtual_pointer.button(
            100,
            button as u32,
            if state == 0 {
                wl_pointer::ButtonState::Released
            } else {
                wl_pointer::ButtonState::Pressed
            },
        );
    }

    pub fn notify_pointer_axis(&self, dx: f64, dy: f64) {
        self.virtual_pointer
            .axis(100, wl_pointer::Axis::HorizontalScroll, dx);
        self.virtual_pointer
            .axis(100, wl_pointer::Axis::VerticalScroll, dy);
    }

    pub fn notify_pointer_axis_discrete(&self, axis: u32, steps: i32) {
        self.virtual_pointer.axis_discrete(
            100,
            if axis == 0 {
                wl_pointer::Axis::VerticalScroll
            } else {
                wl_pointer::Axis::HorizontalScroll
            },
            10.0,
            steps,
        );
    }

    pub fn notify_keyboard_keycode(&self, keycode: i32, state: u32) {
        self.virtual_keyboard.key(100, keycode as u32, state);
    }

    pub fn notify_keyboard_keysym(&self, keysym: i32, state: u32) {
        self.virtual_keyboard.key(100, keysym as u32, state);
    }
}
