use std::os::unix::net::UnixStream;
use std::path::PathBuf;

use std::sync::LazyLock;

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
