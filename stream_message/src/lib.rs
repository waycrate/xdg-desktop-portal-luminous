pub mod error;
use std::{
    io::{Read, Write},
    path::PathBuf,
    sync::LazyLock,
};

use error::Error;
use std::os::unix::net::UnixStream;

use serde::{Deserialize, Serialize};

pub trait SyncCodec {
    fn read_from<T: Read>(stream: &mut T) -> Result<Self, Error>
    where
        Self: std::marker::Sized;
    fn write_to<T: Write>(&self, stream: &mut T) -> Result<(), Error>;
}

pub trait SocketMessage<T>
where
    T: SyncCodec,
{
    fn read_msg(&mut self) -> Result<T, Error>
    where
        T: std::marker::Sized;
    fn write_msg(&mut self, message: T) -> Result<(), Error>;
}

impl<T: SyncCodec> SocketMessage<T> for UnixStream {
    fn read_msg(&mut self) -> Result<T, Error>
    where
        T: std::marker::Sized,
    {
        T::read_from(self)
    }
    fn write_msg(&mut self, message: T) -> Result<(), Error> {
        message.write_to(self)
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
#[serde(tag = "type")]
pub enum Request {
    ScreenShare { monitors: Vec<String> },
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
#[serde(tag = "type")]
pub enum Response {
    Success { index: u32 },
    Cancel,
    Busy,
}

macro_rules! impl_message {
    ($message:ident) => {
        impl SyncCodec for $message {
            fn read_from<T: Read>(stream: &mut T) -> Result<Self, Error> {
                let mut len_bytes = [0; 4];
                stream
                    .read_exact(&mut len_bytes)
                    .map_err(|e| match e.kind() {
                        std::io::ErrorKind::UnexpectedEof => Error::Eof,
                        _ => e.into(),
                    })?;
                let len = u32::from_ne_bytes(len_bytes);

                let mut resp_buf = vec![0; len as usize];
                stream.read_exact(&mut resp_buf)?;
                serde_json::from_slice(&resp_buf).map_err(|e| e.into())
            }

            fn write_to<T: Write>(&self, stream: &mut T) -> Result<(), Error> {
                let body_bytes = serde_json::to_vec(self)?;
                let len_bytes = (body_bytes.len() as u32).to_ne_bytes();
                stream.write_all(&len_bytes)?;
                stream.write_all(&body_bytes)?;
                Ok(())
            }
        }
    };
}

impl_message!(Response);
impl_message!(Request);

pub static SERVER_SOCK: LazyLock<PathBuf> = LazyLock::new(|| {
    let cache_dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or("/tmp".to_string());
    PathBuf::from(cache_dir).join("luminus_selector.sock")
});
