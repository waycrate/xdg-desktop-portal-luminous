mod remotedesktop;
mod request;
mod screencast;
mod screenshot;
mod session;
mod settings;
mod slintbackend;

use remotedesktop::RemoteDesktopBackend;
use screencast::ScreenCastBackend;
use screenshot::ScreenShotBackend;
use settings::{AccentColor, SettingsBackend, SettingsConfig, SETTING_CONFIG};

use std::collections::HashMap;
use std::future::pending;
use zbus::{zvariant, Connection, ConnectionBuilder, SignalContext};

use futures::{
    channel::mpsc::{channel, Receiver},
    SinkExt, StreamExt,
};
use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::Path;

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

fn async_watcher() -> notify::Result<(RecommendedWatcher, Receiver<notify::Result<Event>>)> {
    let (mut tx, rx) = channel(1);

    // Automatically select the best implementation for your platform.
    // You can also access each implementation directly e.g. INotifyWatcher.
    let watcher = RecommendedWatcher::new(
        move |res| {
            futures::executor::block_on(async {
                tx.send(res).await.unwrap();
            })
        },
        Config::default(),
    )?;

    Ok((watcher, rx))
}

async fn async_watch<P: AsRef<Path>>(path: P, connection: Connection) -> notify::Result<()> {
    let (mut watcher, mut rx) = async_watcher()?;

    let signal_context =
        SignalContext::new(&connection, "/org/freedesktop/portal/desktop").unwrap();
    // Add a path to be watched. All files and directories at that path and
    // below will be monitored for changes.
    watcher.watch(path.as_ref(), RecursiveMode::Recursive)?;

    while let Some(res) = rx.next().await {
        match res {
            Ok(_) => {
                let mut config = SETTING_CONFIG.lock().await;
                *config = SettingsConfig::config_from_file();
                let _ = SettingsBackend::setting_changed(
                    &signal_context,
                    "org.freedesktop.appearance".to_string(),
                    "color-scheme".to_string(),
                    config.get_color_scheme().into(),
                )
                .await;
                let _ = SettingsBackend::setting_changed(
                    &signal_context,
                    "org.freedesktop.appearance".to_string(),
                    "accent-color".to_string(),
                    AccentColor {
                        color: config.get_accent_color(),
                    }
                    .into(),
                )
                .await;
            }
            Err(e) => println!("watch error: {:?}", e),
        }
    }

    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    std::env::set_var("RUST_LOG", "xdg-desktop-protal-luminous=info");
    tracing_subscriber::fmt().init();
    tracing::info!("luminous Start");

    let conn = ConnectionBuilder::session()?
        .name("org.freedesktop.impl.portal.desktop.luminous")?
        .serve_at("/org/freedesktop/portal/desktop", ScreenShotBackend)?
        .serve_at("/org/freedesktop/portal/desktop", ScreenCastBackend)?
        .serve_at("/org/freedesktop/portal/desktop", RemoteDesktopBackend)?
        .serve_at("/org/freedesktop/portal/desktop", SettingsBackend)?
        .build()
        .await?;

    tokio::spawn(async move {
        let Ok(home) = std::env::var("HOME") else {
            return;
        };
        let config_path = std::path::Path::new(home.as_str())
            .join(".config")
            .join("xdg-desktop-portal-luminous");
        async_watch(config_path, conn).await.unwrap()
    });

    pending::<()>().await;
    Ok(())
}
