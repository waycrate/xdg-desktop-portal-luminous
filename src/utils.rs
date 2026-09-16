use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use xkbcommon::xkb::{
    CONTEXT_NO_FLAGS, Context, KEYMAP_COMPILE_NO_FLAGS, KEYMAP_FORMAT_TEXT_V1, Keymap, State,
};

use std::sync::LazyLock;
use std::{ffi::CString, fs::File, io::Write};

use stream_message::{Request, Response, SERVER_SOCK, SocketMessage};

pub static USER_RUNNING_DIR: LazyLock<PathBuf> = LazyLock::new(|| {
    let cache_dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or("/tmp".to_string());
    PathBuf::from(cache_dir)
});

pub static HEADLESS_START: LazyLock<bool> = LazyLock::new(|| {
    if std::env::var("WLR_BACKENDS").is_ok_and(|v| v == "headless") {
        return true;
    }
    std::env::var("LUMIOUS_HEADLESS")
        .map(|v| v == "1")
        .unwrap_or(false)
});

pub fn get_selection_from_socket(monitors: Vec<String>) -> zbus::fdo::Result<u32> {
    let mut stream = UnixStream::connect(SERVER_SOCK.clone())
        .map_err(|_| zbus::fdo::Error::Failed("Cannot connect to socket".to_owned()))?;
    stream
        .write_msg(Request::ScreenShare { monitors })
        .map_err(|_| zbus::fdo::Error::Failed("Cannot send message to socket".to_owned()))?;
    let response: Response = stream
        .read_msg()
        .map_err(|_| zbus::fdo::Error::Failed("Cannot read msg from socket".to_owned()))?;

    match response {
        Response::Success { index } => Ok(index),
        Response::Busy => Err(zbus::fdo::Error::Failed(
            "now other program is selecting now".to_owned(),
        )),
        Response::Cancel => Err(zbus::fdo::Error::Failed("Cancelled".to_owned())),
    }
}

pub static XDG_CONFIG_HOME: LazyLock<Option<PathBuf>> = LazyLock::new(|| {
    if let Ok(xdg_config_home_env) = std::env::var("XDG_CONFIG_HOME")
        && let xdg_config_home = PathBuf::from(xdg_config_home_env)
        && xdg_config_home.is_absolute()
    {
        tracing::warn!(
            "Ignoring relative XDG_CONFIG_HOME for Background autostart: {}",
            xdg_config_home.display()
        );
        return Some(xdg_config_home);
    }
    let home = std::env::var("HOME").ok()?;
    Some(PathBuf::from(&home).join(".config"))
});

#[derive(Debug, Clone)]
pub struct InputEvent {
    pub session_handle: String,
    pub request: InputRequest,
}

#[derive(Debug, Clone, Copy)]
pub enum InputRequest {
    PointerMotion { dx: f64, dy: f64 },
    PointerMotionAbsolute { x: f64, y: f64 },
    PointerButton { button: i32, state: u32 },
    PointerAxis { dx: f64, dy: f64, finish: bool },
    PointerAxisDiscrete { axis: u32, steps: i32 },
    KeyboardKeycode { keycode: i32, state: u32 },
    KeyboardKeysym { keysym: i32, state: u32 },
    TouchMotion { slot: u32, x: f64, y: f64 },
    TouchDown { slot: u32, x: f64, y: f64 },
    TouchUp { slot: u32 },
    Exit,
}
// NOTE: always read https://github.com/torvalds/linux/blob/master/include/uapi/linux/input-event-codes.h
pub const BTN_LEFT: u32 = 0x110;
pub const BTN_RIGHT: u32 = 0x111;
pub const BTN_MIDDLE: u32 = 0x112;
//const PAD_LEFT: u32 = 0x222;
pub const PAD_RIGHT: u32 = 0x223;
pub fn from_icedmouse_to_u32(mouse: iced::mouse::Button) -> u32 {
    match mouse {
        iced::mouse::Button::Right => BTN_LEFT,
        iced::mouse::Button::Middle => BTN_MIDDLE,
        iced::mouse::Button::Other(code) => code as u32,
        _ => BTN_LEFT,
    }
}

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
