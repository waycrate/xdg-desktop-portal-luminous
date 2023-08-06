use std::future::pending;
use zbus::ConnectionBuilder;

mod screenshot;
mod slintbackend;

use screenshot::ShanaShot;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    std::env::set_var("RUST_LOG", "xdg-desktop-protal-wlrrust=info");
    tracing_subscriber::fmt().init();
    tracing::info!("wlrrust Start");
    let _conn = ConnectionBuilder::session()?
        .name("org.freedesktop.impl.portal.desktop.wlrrust")?
        .serve_at("/org/freedesktop/portal/desktop", ShanaShot::new())?
        .build()
        .await?;

    pending::<()>().await;
    Ok(())
}
