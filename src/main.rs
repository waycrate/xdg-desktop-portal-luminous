mod access;
mod backend;
mod gui;
mod remotedesktop;
mod request;
mod screencast;
mod screenshot;
mod session;
mod settings;
mod utils;

use std::collections::HashMap;
use zbus::zvariant;
mod pipewirethread;

const PORTAL_RESPONSE_SUCCESS: u32 = 0;
const PORTAL_RESPONSE_CANCELLED: u32 = 1;
const PORTAL_RESPONSE_OTHER: u32 = 2;

#[derive(zvariant::Type)]
#[zvariant(signature = "(ua{sv})")]
enum PortalResponse<T: zvariant::Type + serde::Serialize> {
    Success(T),
    Cancelled,
    Other,
}

impl<T: zvariant::Type + serde::Serialize> serde::Serialize for PortalResponse<T> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Success(res) => (PORTAL_RESPONSE_SUCCESS, res).serialize(serializer),
            Self::Cancelled => (
                PORTAL_RESPONSE_CANCELLED,
                HashMap::<String, zvariant::Value>::new(),
            )
                .serialize(serializer),
            Self::Other => (
                PORTAL_RESPONSE_OTHER,
                HashMap::<String, zvariant::Value>::new(),
            )
                .serialize(serializer),
        }
    }
}

fn main() -> anyhow::Result<()> {
    let support_toplevel_capture = libwayshot::WayshotConnection::new()
        .map(|conn| conn.toplevel_capture_support())
        .unwrap_or(false);
    let _ = gui::gui(support_toplevel_capture)?;
    Ok(())
}
