use enumflags2::{bitflags, BitFlags};
use zbus::{interface, zvariant::OwnedObjectPath, SignalContext};

use serde::{Deserialize, Serialize};
use serde_repr::{Deserialize_repr, Serialize_repr};
use zbus::zvariant::Type;

use once_cell::sync::Lazy;

use std::sync::Arc;
use tokio::sync::Mutex;

use crate::{
    remotedesktop::{remove_remote_session, SelectDevicesOptions},
    screencast::{remove_cast_session, SelectSourcesOptions},
};

pub static SESSIONS: Lazy<Arc<Mutex<Vec<Session>>>> =
    Lazy::new(|| Arc::new(Mutex::new(Vec::new())));

pub async fn append_session(session: Session) {
    let mut sessions = SESSIONS.lock().await;
    sessions.push(session)
}

pub async fn remove_session(session: &Session) {
    let mut sessions = SESSIONS.lock().await;
    let Some(index) = sessions
        .iter()
        .position(|the_session| the_session.handle_path == session.handle_path)
    else {
        return;
    };
    remove_cast_session(&session.handle_path.to_string()).await;
    remove_remote_session(&session.handle_path.to_string()).await;
    sessions.remove(index);
}

#[bitflags]
#[derive(Serialize, Default, Deserialize, PartialEq, Eq, Copy, Clone, Debug, Type)]
#[repr(u32)]
/// A bit flag for the available sources to record.
pub enum SourceType {
    #[default]
    /// A monitor.
    Monitor,
    /// A specific window
    Window,
    /// Virtual
    Virtual,
}

#[bitflags]
#[derive(Serialize_repr, Deserialize_repr, PartialEq, Eq, Debug, Copy, Clone, Type, Default)]
#[repr(u32)]
/// A bit flag for the possible cursor modes.
pub enum CursorMode {
    #[default]
    /// The cursor is not part of the screen cast stream.
    Hidden = 1,
    /// The cursor is embedded as part of the stream buffers.
    Embedded = 2,
    /// The cursor is not part of the screen cast stream, but sent as PipeWire
    /// stream metadata.
    Metadata = 4,
}

// Remote
#[bitflags]
#[derive(Serialize_repr, Deserialize_repr, PartialEq, Eq, Debug, Copy, Clone, Type, Default)]
#[repr(u32)]
/// A bit flag for the possible cursor modes.
pub enum DeviceType {
    #[default]
    /// The cursor is not part of the screen cast stream.
    Keyboard = 1,
    /// The cursor is embedded as part of the stream buffers.
    Pointer = 2,
    /// The cursor is not part of the screen cast stream, but sent as PipeWire
    /// stream metadata.
    TouchScreen = 4,
}

impl CursorMode {
    pub fn show_cursor(&self) -> bool {
        !matches!(self, CursorMode::Hidden)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionType {
    ScreenCast,
    Remote,
}

#[derive(Default, Serialize, Deserialize, PartialEq, Eq, Debug, Copy, Clone, Type)]
#[repr(u32)]
/// Persistence mode for a screencast session.
pub enum PersistMode {
    #[default]
    /// Do not persist.
    DoNot = 0,
    /// Persist while the application is running.
    Application = 1,
    /// Persist until explicitly revoked.
    ExplicitlyRevoked = 2,
}

#[derive(Debug, Clone)]
// TODO: when is remote?
pub struct Session {
    pub session_type: SessionType,
    pub handle_path: OwnedObjectPath,
    pub source_type: BitFlags<SourceType>,
    pub multiple: bool,
    pub cursor_mode: CursorMode,
    pub persist_mode: PersistMode,

    pub device_type: BitFlags<DeviceType>,
}

impl Session {
    pub fn new<P: Into<OwnedObjectPath>>(path: P, session_type: SessionType) -> Self {
        Self {
            session_type,
            handle_path: path.into(),
            source_type: SourceType::Monitor.into(),
            multiple: false,
            cursor_mode: CursorMode::Hidden,
            persist_mode: PersistMode::DoNot,
            device_type: DeviceType::Keyboard.into(),
        }
    }
    pub fn set_screencast_options(&mut self, options: SelectSourcesOptions) {
        if let Some(types) = options.types {
            self.source_type = types;
        }
        self.multiple = options.multiple.is_some_and(|content| content);
        if let Some(cursormode) = options.cursor_mode {
            self.cursor_mode = cursormode;
        }
        if let Some(persist_mode) = options.persist_mode {
            self.persist_mode = persist_mode;
        }
    }

    pub fn set_remote_options(&mut self, options: SelectDevicesOptions) {
        if let Some(types) = options.types {
            self.device_type = types;
        }
        if let Some(persist_mode) = options.persist_mode {
            self.persist_mode = persist_mode;
        }
    }
}

#[interface(name = "org.freedesktop.impl.portal.Session")]
impl Session {
    async fn close(
        &self,
        #[zbus(signal_context)] cxts: SignalContext<'_>,
        #[zbus(object_server)] server: &zbus::ObjectServer,
    ) -> zbus::fdo::Result<()> {
        server
            .remove::<Self, &OwnedObjectPath>(&self.handle_path)
            .await?;
        remove_session(self).await;
        Self::closed(&cxts, "Closed").await?;
        Ok(())
    }

    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        2
    }

    #[zbus(signal)]
    async fn closed(signal_ctxt: &SignalContext<'_>, message: &str) -> zbus::Result<()>;
}
