use super::state::AppData;
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle, delegate_noop,
    globals::GlobalListContents,
    protocol::{wl_keyboard, wl_registry, wl_seat::WlSeat, wl_shm::WlShm},
};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1,
    zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
};

use std::{ffi::CString, fs::File, io::Write, os::fd::AsFd, path::PathBuf};
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1,
    zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
};
use xkbcommon::xkb::{
    CONTEXT_NO_FLAGS, Context, KEYMAP_COMPILE_NO_FLAGS, KEYMAP_FORMAT_TEXT_V1, Keymap, State,
};

pub fn init_xkb_objects() -> (Context, Keymap, State) {
    let context = Context::new(CONTEXT_NO_FLAGS);
    let keymap = Keymap::new_from_names(&context, "", "", "us", "", None, KEYMAP_COMPILE_NO_FLAGS)
        .expect("xkbcommon keymap panicked!");
    let state = State::new(&keymap);
    (context, keymap, state)
}

pub fn get_keymap_as_file(state: &State) -> (File, u32) {
    let keymap = state.get_keymap().get_as_string(KEYMAP_FORMAT_TEXT_V1);
    let keymap = CString::new(keymap).expect("Keymap should not contain interior nul bytes");
    let keymap = keymap.as_bytes_with_nul();
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let mut file = tempfile::tempfile_in(dir).expect("File could not be created!");
    file.write_all(keymap).unwrap();
    file.flush().unwrap();
    (file, keymap.len() as u32)
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for AppData {
    fn event(
        _state: &mut Self,
        _proxy: &wl_registry::WlRegistry,
        _event: <wl_registry::WlRegistry as Proxy>::Event,
        _data: &GlobalListContents,
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for AppData {
    fn event(
        state: &mut Self,
        _proxy: &wl_keyboard::WlKeyboard,
        event: <wl_keyboard::WlKeyboard as Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        if let wl_keyboard::Event::Keymap { format, fd, size } = event {
            match format.into_result() {
                Ok(wl_keyboard::KeymapFormat::XkbV1) => {
                    state.virtual_keyboard.keymap(
                        wl_keyboard::KeymapFormat::XkbV1.into(),
                        fd.as_fd(),
                        size,
                    );
                    if let Some(xkb_keymap) = unsafe {
                        Keymap::new_from_fd(
                            &state.xkb_context,
                            fd,
                            size as usize,
                            wl_keyboard::KeymapFormat::XkbV1.into(),
                            KEYMAP_COMPILE_NO_FLAGS,
                        )
                    }
                    .expect("Failed to create XKB keymap from file descriptor")
                    {
                        state.xkb_state = State::new(&xkb_keymap);
                        state.xkb_keymap = xkb_keymap;
                    }
                }
                _ => tracing::error!("Cannot obtain valid keymap format from keymap event"),
            }
        }
    }
}

delegate_noop!(AppData: ignore ZwpVirtualKeyboardManagerV1);
delegate_noop!(AppData: ignore ZwpVirtualKeyboardV1);
delegate_noop!(AppData: ignore ZwlrVirtualPointerManagerV1);
delegate_noop!(AppData: ignore ZwlrVirtualPointerV1);
delegate_noop!(AppData: ignore WlSeat);
delegate_noop!(AppData: ignore WlShm);
delegate_noop!(AppData: ignore wl_registry::WlRegistry);
