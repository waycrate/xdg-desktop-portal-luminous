use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1;

use wayland_client::{
    DispatchError,
    globals::{BindError, GlobalError},
    protocol::{wl_keyboard::KeyState, wl_pointer},
};
use wayland_protocols_wlr::virtual_pointer::v1::client::zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1;

use enumflags2::{BitFlag, BitFlags, bitflags};
use thiserror::Error;
use xkbcommon::xkb::{Context, Keycode, Keymap, Keysym, STATE_LAYOUT_EFFECTIVE, State};

const LEFT_SHIFT: i32 = 42;
const ALTGR: i32 = 100;
// This struct represents the state of our app. This simple app does not
// need any state, by this type still supports the `Dispatch` implementations.
pub struct AppData {
    pub(crate) virtual_keyboard: ZwpVirtualKeyboardV1,
    pub(crate) virtual_pointer: ZwlrVirtualPointerV1,
    pub(crate) mods: u32,
    pub(crate) xkb_context: Context,
    pub(crate) xkb_keymap: Keymap,
    pub(crate) xkb_state: State,
    x: u32,
    y: u32,
    space_width: u32,
    space_height: u32,
}

impl AppData {
    pub fn new(
        virtual_keyboard: ZwpVirtualKeyboardV1,
        virtual_pointer: ZwlrVirtualPointerV1,
        xkb_context: Context,
        xkb_keymap: Keymap,
        xkb_state: State,
        x: u32,
        y: u32,
        space_width: u32,
        space_height: u32,
    ) -> Self {
        Self {
            virtual_keyboard,
            virtual_pointer,
            mods: Modifiers::empty().bits(),
            xkb_context,
            xkb_keymap,
            xkb_state,
            x,
            y,
            space_width,
            space_height,
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
    InitFailedConnection(String),
    #[error("Error during queue")]
    FailedDuringQueue(#[from] DispatchError),
    #[error("GlobalError")]
    GlobalError(#[from] GlobalError),
    #[error("BindError")]
    BindFailed(#[from] BindError),
}

#[bitflags]
#[derive(PartialEq, Eq, Copy, Clone)]
#[repr(u32)]
// Modifiers passed to the virtual_keyboard protocol. They are based on
// wayland's wl_keyboard, which doesn't document them.
enum Modifiers {
    Shift = 1,
    CapsLock = 2,
    Ctrl = 4,
    Alt = 8,
    Super = 64,
    AltGr = 128,
}

impl AppData {
    // Keycode mappings as can be found in the file `/usr/include/linux/input-event-codes.h`.
    fn get_modifier_from_keycode(&self, keycode: i32) -> Option<Modifiers> {
        match keycode {
            LEFT_SHIFT | 54 => Some(Modifiers::Shift), // left and right Shift
            58 => Some(Modifiers::CapsLock),
            29 | 97 => Some(Modifiers::Ctrl), // left and right Ctrl
            56 => Some(Modifiers::Alt),
            125 | 126 => Some(Modifiers::Super), // left and right Super
            ALTGR => Some(Modifiers::AltGr),
            _ => None,
        }
    }

    fn get_keycode_from_keysym(&self, keysym: Keysym) -> Option<(u32, u32)> {
        const EVDEV_OFFSET: u32 = 8;
        let layout = self.xkb_state.serialize_layout(STATE_LAYOUT_EFFECTIVE);
        let max_keycode = self.xkb_keymap.max_keycode();
        for keycode in (self.xkb_keymap.min_keycode().raw()..max_keycode.raw()).map(Keycode::new) {
            let n_levels = self.xkb_keymap.num_levels_for_key(keycode, layout);
            for level in 0..n_levels {
                let syms = self
                    .xkb_keymap
                    .key_get_syms_by_level(keycode, layout, level);
                for sym in syms {
                    if *sym == keysym {
                        return Some((u32::from(keycode) - EVDEV_OFFSET, level));
                    }
                }
            }
        }
        None
    }

    pub fn notify_pointer_motion(&self, dx: f64, dy: f64) {
        self.virtual_pointer.motion(10, dx, dy);
    }

    pub fn notify_pointer_motion_absolute(&self, x: f64, y: f64) {
        let x = x as u32 + self.x;
        let y = y as u32 + self.y;
        self.virtual_pointer
            .motion_absolute(10, x, y, self.space_width, self.space_height);
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

    pub fn notify_keyboard_keycode(&mut self, keycode: i32, state: u32) {
        let pressed_key: u32 = KeyState::Pressed.into();
        match self.get_modifier_from_keycode(keycode) {
            // Caps lock is managed differently as it's the only
            // modifier key that is still active after being released
            Some(Modifiers::CapsLock) => {
                if state == pressed_key {
                    self.mods ^= BitFlags::from_flag(Modifiers::CapsLock).bits();
                    self.virtual_keyboard.modifiers(self.mods, 0, 0, 0)
                }
            }
            // Other modifier keys
            Some(modifier) => {
                if state == pressed_key {
                    self.mods |= BitFlags::from_flag(modifier).bits()
                } else {
                    self.mods &= !BitFlags::from_flag(modifier).bits()
                }
                self.virtual_keyboard.modifiers(self.mods, 0, 0, 0)
            }
            // non-modifier key
            _ => self.virtual_keyboard.key(100, keycode as u32, state),
        }
    }

    pub fn notify_keyboard_keysym(&mut self, keysym: i32, state: u32) {
        if let Some((keycode, level)) = self.get_keycode_from_keysym(Keysym::new(keysym as u32)) {
            match level {
                0 => {}
                1 => self.notify_keyboard_keycode(LEFT_SHIFT, state),
                2 => self.notify_keyboard_keycode(ALTGR, state),
                _ => tracing::warn!(
                    "Received unsupported key level during keysym conversion: {}",
                    level
                ),
            }
            self.notify_keyboard_keycode(keycode as i32, state);
        } else {
            tracing::warn!("Could not find keycode for keysym: {}", keysym);
        }
    }

    pub fn notify_touch_down(&mut self, _slot: u32, _x: f64, _y: f64) {
        tracing::debug!("NotifyTouchDown: touch events are currently unsupported");
    }

    pub fn notify_touch_motion(&mut self, _slot: u32, _x: f64, _y: f64) {
        tracing::debug!("NotifyTouchMotion: touch events are currently unsupported");
    }

    pub fn notify_touch_up(&mut self, _slot: u32) {
        tracing::debug!("NotifyTouchUp: touch events are currently unsupported");
    }
}
