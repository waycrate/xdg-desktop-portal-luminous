mod request;
mod screencast;
mod screenshot;
mod session;
mod slintbackend;

use screencast::ScreenCast;
use screenshot::ShanaShot;

use std::collections::HashMap;
use std::future::pending;
use zbus::{zvariant, ConnectionBuilder};

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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    std::env::set_var("RUST_LOG", "xdg-desktop-protal-wlrrust=info");
    tracing_subscriber::fmt().init();
    tracing::info!("wlrrust Start");

    let _conn = ConnectionBuilder::session()?
        .name("org.freedesktop.impl.portal.desktop.wlrrust")?
        .serve_at("/org/freedesktop/portal/desktop", ShanaShot::new())?
        .serve_at("/org/freedesktop/portal/desktop", ScreenCast)?
        .build()
        .await?;

    pending::<()>().await;
    Ok(())
}
