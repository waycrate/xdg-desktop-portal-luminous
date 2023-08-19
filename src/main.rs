mod request;
mod screencast;
mod screenshot;
mod session;
mod slintbackend;

use screencast::ScreenCast;
use screenshot::ShanaShot;

use std::future::pending;
use zbus::ConnectionBuilder;

mod pipewirethread;


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
