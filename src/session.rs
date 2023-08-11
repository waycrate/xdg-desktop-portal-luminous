use enumflags2::bitflags;
use zbus::{dbus_interface, SignalContext};

use serde::{Deserialize, Serialize};
use zbus::zvariant::Type;

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
#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Copy, Clone, Type, Default)]
#[repr(u32)]
/// A bit flag for the possible cursor modes.
pub enum CursorMode {
    #[default]
    /// The cursor is not part of the screen cast stream.
    Hidden,
    /// The cursor is embedded as part of the stream buffers.
    Embedded,
    /// The cursor is not part of the screen cast stream, but sent as PipeWire
    /// stream metadata.
    Metadata,
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

#[derive(Debug, Default)]
// TODO: when is remote?
pub struct Session {
    pub source_type: SourceType,
    pub cursor_mode: CursorMode,
    pub persist_mode: PersistMode,
}

#[dbus_interface(name = "org.freedesktop.impl.portal.Session")]
impl Session {
    async fn close(
        &self,
        #[zbus(signal_context)] cxts: SignalContext<'_>,
    ) -> zbus::fdo::Result<()> {
        Self::closed(&cxts, "Closed").await?;
        Ok(())
    }

    #[dbus_interface(property, name = "version")]
    fn version(&self) -> u32 {
        2
    }

    #[dbus_interface(signal)]
    async fn closed(signal_ctxt: &SignalContext<'_>, message: &str) -> zbus::Result<()>;
}
