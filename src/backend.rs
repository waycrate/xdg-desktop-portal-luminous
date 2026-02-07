use crate::access::AccessBackend;
use crate::remotedesktop::RemoteDesktopBackend;
use crate::screencast::ScreenCastBackend;
use crate::screenshot::ScreenShotBackend;
use crate::settings::{AccentColor, SETTING_CONFIG, SettingsBackend, SettingsConfig};

use crate::dialog::{CopySelect, Message};
use futures::{
    SinkExt, StreamExt,
    channel::mpsc::{Receiver, Sender, channel},
};
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::future::pending;
use std::path::Path;
use zbus::{Connection, connection, object_server::SignalEmitter};

use std::sync::OnceLock;

static SESSION: OnceLock<zbus::Connection> = OnceLock::new();

async fn get_connection() -> zbus::Connection {
    if let Some(cnx) = SESSION.get() {
        cnx.clone()
    } else {
        panic!("Cannot get cnx");
    }
}

async fn set_connection(connection: Connection) {
    SESSION.set(connection).expect("Cannot set a OnceLock");
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

async fn update_settings<'a>(signal_context: &SignalEmitter<'a>) {
    let mut config = SETTING_CONFIG.lock().await;
    *config = SettingsConfig::config_from_file();
    let _ = SettingsBackend::setting_changed(
        signal_context,
        "org.freedesktop.appearance".to_string(),
        "color-scheme".to_string(),
        config.get_color_scheme().into(),
    )
    .await;
    let _ = SettingsBackend::setting_changed(
        signal_context,
        "org.freedesktop.appearance".to_string(),
        "accent-color".to_string(),
        AccentColor::new(config.get_accent_color())
            .try_into()
            .unwrap(),
    )
    .await;
    let _ = SettingsBackend::setting_changed(
        signal_context,
        "org.freedesktop.appearance".to_string(),
        "contrast".to_string(),
        config.get_contrast().into(),
    )
    .await;
    let _ = SettingsBackend::setting_changed(
        signal_context,
        "org.freedesktop.appearance".to_string(),
        "reduced-motion".to_string(),
        config.get_reduced_motion().into(),
    )
    .await;
}

async fn async_watch<P: AsRef<Path>>(path: P) -> notify::Result<()> {
    let connection = get_connection().await;
    let (mut watcher, mut rx) = async_watcher()?;

    let signal_context =
        SignalEmitter::new(&connection, "/org/freedesktop/portal/desktop").unwrap();
    // Add a path to be watched. All files and directories at that path and
    // below will be monitored for changes.
    watcher.watch(path.as_ref(), RecursiveMode::Recursive)?;

    while let Some(res) = rx.next().await {
        match res {
            Ok(Event {
                kind: EventKind::Modify(_),
                ..
            })
            | Ok(Event {
                kind: EventKind::Create(_),
                ..
            }) => {
                update_settings(&signal_context).await;
            }
            Err(e) => println!("watch error: {e:?}"),
            _ => {}
        }
    }

    Ok(())
}

use crate::remotedesktop;

pub async fn backend(
    sender: Sender<Message>,
    receiver: Receiver<CopySelect>,
    receiver_cast: Receiver<CopySelect>,
) -> anyhow::Result<()> {
    let toplevel_capture_support = libwayshot::WayshotConnection::new()
        .map(|conn| conn.toplevel_capture_support())
        .unwrap_or(false);
    let conn = connection::Builder::session()?
        .name("org.freedesktop.impl.portal.desktop.luminous")?
        .serve_at("/org/freedesktop/portal/desktop", AccessBackend)?
        .serve_at(
            "/org/freedesktop/portal/desktop",
            ScreenShotBackend {
                sender: sender.clone(),
                receiver,
            },
        )?
        .serve_at(
            "/org/freedesktop/portal/desktop",
            ScreenCastBackend {
                toplevel_capture_support,
                sender,
                receiver: receiver_cast,
            },
        )?
        .serve_at("/org/freedesktop/portal/desktop", RemoteDesktopBackend)?
        .serve_at("/org/freedesktop/portal/desktop", SettingsBackend)?
        .build()
        .await?;

    set_connection(conn).await;
    tokio::spawn(async {
        let Ok(home) = std::env::var("HOME") else {
            return;
        };
        let config_path = std::path::Path::new(home.as_str())
            .join(".config")
            .join("xdg-desktop-portal-luminous");
        if let Err(e) = async_watch(config_path).await {
            tracing::info!("Maybe file is not exist, error: {e}");
        }
    });

    let receiver = remotedesktop::get_input_receiver();
    tokio::task::spawn_blocking(move || {
        loop {
            let event = receiver.lock().unwrap().recv().unwrap();
            tokio::spawn(remotedesktop::handle_input_event(event));
        }
    });

    let connection = get_connection().await;

    let signal_context =
        SignalEmitter::new(&connection, "/org/freedesktop/portal/desktop").unwrap();
    update_settings(&signal_context).await;
    pending::<()>().await;

    Ok(())
}
