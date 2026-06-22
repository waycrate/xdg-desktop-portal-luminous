use crate::access::AccessBackend;
use crate::background::{BackgroundBackend, PendingBackgroundResponses};
use crate::clipboard::Clipboard;
use crate::dialog::{CopySelect, Message};
use crate::input_capture::InputCapture;
use crate::remotedesktop::RemoteDesktopBackend;
use crate::screencast::ScreenCastBackend;
use crate::screenshot::ScreenShotBackend;
use crate::settings::XDG_CONFIG_HOME_FILE;
use crate::settings::{AccentColor, SETTING_CONFIG, SettingsBackend, SettingsConfig};
use futures::{
    SinkExt, StreamExt,
    channel::mpsc::{Receiver, Sender, UnboundedReceiver, channel},
};
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::{HashMap, HashSet};
use std::future::pending;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, mpsc as tokio_mpsc};
use tokio::time::MissedTickBehavior;
use zbus::{Connection, connection, object_server::SignalEmitter};

use std::sync::OnceLock;

static SESSION: OnceLock<zbus::Connection> = OnceLock::new();
const SYSTEMD_SIGNAL_RETRY_BACKOFF: Duration = Duration::from_secs(5);

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
    receiver_background: UnboundedReceiver<CopySelect>,
) -> anyhow::Result<()> {
    let toplevel_capture_support = libwayshot::WayshotConnection::new()
        .map(|conn| conn.toplevel_capture_support())
        .unwrap_or(false);
    let pending_background_responses: PendingBackgroundResponses =
        Arc::new(Mutex::new(Default::default()));
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
                sender: sender.clone(),
                receiver: receiver_cast,
            },
        )?
        .serve_at(
            "/org/freedesktop/portal/desktop",
            BackgroundBackend {
                sender,
                pending_responses: pending_background_responses.clone(),
            },
        )?
        .serve_at("/org/freedesktop/portal/desktop", RemoteDesktopBackend)?
        .serve_at("/org/freedesktop/portal/desktop", SettingsBackend)?
        .serve_at("/org/freedesktop/portal/desktop", InputCapture)?
        .serve_at("/org/freedesktop/portal/desktop", Clipboard)?
        .build()
        .await?;

    set_connection(conn).await;
    tokio::spawn(crate::background::route_background_dialog_responses(
        receiver_background,
        pending_background_responses,
    ));

    let background_connection = get_connection().await;
    tokio::spawn(async move {
        if let Err(e) = watch_background_applications(background_connection).await {
            tracing::info!("Cannot watch systemd app scopes: {e}");
        }
    });

    tokio::spawn(async {
        let Some(config_path) = XDG_CONFIG_HOME_FILE.clone() else {
            tracing::info!("File not exist under $XDG_CONFIG_HOME/xdg-desktop-portal-luminous");
            return;
        };
        if let Err(e) = async_watch(config_path).await {
            tracing::info!("Maybe file is not exist, error: {e}");
        }
    });

    let receiver = remotedesktop::get_input_receiver();
    std::thread::spawn(move || {
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

async fn watch_background_applications(connection: Connection) -> zbus::Result<()> {
    let mut app_scope_tracker = AppScopeTracker::default();
    let mut app_scope_tracker_seeded = false;
    let (systemd_event_sender, mut systemd_event_receiver) = tokio_mpsc::channel(32);
    tokio::spawn(forward_systemd_scope_signals(
        connection.clone(),
        systemd_event_sender,
    ));

    let signal_context =
        SignalEmitter::new(&connection, "/org/freedesktop/portal/desktop").unwrap();
    let mut last_windowed_app_ids = collect_windowed_app_ids().unwrap_or_default();
    let mut window_poll = tokio::time::interval(Duration::from_secs(10));
    window_poll.set_missed_tick_behavior(MissedTickBehavior::Skip);
    window_poll.tick().await;

    loop {
        tokio::select! {
            Some(event) = systemd_event_receiver.recv() => {
                match event {
                    SystemdMonitorEvent::Signal { signal, message } => {
                        if let Err(e) = emit_running_applications_changed_for_app_scope(
                            &signal_context,
                            &mut app_scope_tracker,
                            signal,
                            &message,
                        )
                        .await {
                            tracing::warn!(
                                "Failed to emit RunningApplicationsChanged for systemd app scope signal: {e}"
                            );
                        }
                    }
                    SystemdMonitorEvent::Reconcile(units) => {
                        if !app_scope_tracker_seeded {
                            app_scope_tracker = AppScopeTracker::from_units(units);
                            app_scope_tracker_seeded = true;
                            continue;
                        }

                        let emit_result = if app_scope_tracker.reconcile_units(units) {
                            BackgroundBackend::running_applications_changed(&signal_context).await
                        } else {
                            Ok(())
                        };
                        if let Err(e) = emit_result {
                            tracing::warn!(
                                "Failed to emit RunningApplicationsChanged after systemd app scope reconciliation: {e}"
                            );
                        }
                    }
                }
            }
            _ = window_poll.tick() => {
                emit_running_applications_changed_for_windowed_apps(
                    &signal_context,
                    &mut last_windowed_app_ids,
                )
                .await;
            }
        }
    }
}

async fn forward_systemd_scope_signals(
    connection: Connection,
    sender: tokio_mpsc::Sender<SystemdMonitorEvent>,
) {
    loop {
        if sender.is_closed() {
            return;
        }

        match forward_systemd_scope_signals_once(&connection, &sender).await {
            Ok(()) if sender.is_closed() => return,
            Ok(()) => {
                tracing::warn!(
                    "Systemd app scope signal stream ended; retrying in {} seconds",
                    SYSTEMD_SIGNAL_RETRY_BACKOFF.as_secs()
                );
            }
            Err(e) => {
                tracing::warn!(
                    "Cannot watch systemd app scope signals: {e}; retrying in {} seconds",
                    SYSTEMD_SIGNAL_RETRY_BACKOFF.as_secs()
                );
            }
        }

        tokio::time::sleep(SYSTEMD_SIGNAL_RETRY_BACKOFF).await;
    }
}

async fn forward_systemd_scope_signals_once(
    connection: &Connection,
    sender: &tokio_mpsc::Sender<SystemdMonitorEvent>,
) -> zbus::Result<()> {
    let systemd = crate::systemd::Systemd1Proxy::new(connection).await?;
    let proxy = zbus::Proxy::new(
        connection,
        "org.freedesktop.systemd1",
        "/org/freedesktop/systemd1",
        "org.freedesktop.systemd1.Manager",
    )
    .await?;

    let mut unit_new = proxy.receive_signal("UnitNew").await?;
    let mut unit_removed = proxy.receive_signal("UnitRemoved").await?;
    systemd.subscribe().await?;

    let units = systemd.list_units().await?;
    if sender
        .send(SystemdMonitorEvent::Reconcile(units))
        .await
        .is_err()
    {
        return Ok(());
    }

    loop {
        tokio::select! {
            message = unit_new.next() => {
                match message {
                    Some(message) => {
                        if sender.send(SystemdMonitorEvent::Signal {
                            signal: UnitSignal::New,
                            message,
                        }).await.is_err() {
                            return Ok(());
                        }
                    }
                    None => return Ok(()),
                }
            }
            message = unit_removed.next() => {
                match message {
                    Some(message) => {
                        if sender.send(SystemdMonitorEvent::Signal {
                            signal: UnitSignal::Removed,
                            message,
                        }).await.is_err() {
                            return Ok(());
                        }
                    }
                    None => return Ok(()),
                }
            }
        }
    }
}

async fn emit_running_applications_changed_for_windowed_apps(
    signal_context: &SignalEmitter<'_>,
    last_windowed_app_ids: &mut HashSet<String>,
) {
    let Some(windowed_app_ids) = collect_windowed_app_ids() else {
        return;
    };

    if !update_windowed_app_ids(last_windowed_app_ids, windowed_app_ids) {
        return;
    }

    if let Err(e) = BackgroundBackend::running_applications_changed(signal_context).await {
        tracing::warn!("Failed to emit RunningApplicationsChanged for window changes: {e}");
    }
}

fn collect_windowed_app_ids() -> Option<HashSet<String>> {
    let mut wayshot_connection = match libwayshot::WayshotConnection::new() {
        Ok(connection) => connection,
        Err(e) => {
            tracing::warn!("Cannot get Wayland toplevels for background monitor: {e:?}");
            return None;
        }
    };

    if let Err(e) = wayshot_connection.refresh_toplevels() {
        tracing::warn!("Cannot refresh Wayland toplevels for background monitor: {e:?}");
        return None;
    }

    Some(
        wayshot_connection
            .get_all_toplevels()
            .iter()
            .filter(|top_level| !top_level.app_id.is_empty())
            .map(|top_level| top_level.app_id.clone())
            .collect(),
    )
}

fn update_windowed_app_ids(
    last_windowed_app_ids: &mut HashSet<String>,
    windowed_app_ids: HashSet<String>,
) -> bool {
    if windowed_app_ids == *last_windowed_app_ids {
        false
    } else {
        *last_windowed_app_ids = windowed_app_ids;
        true
    }
}

#[derive(Clone, Copy)]
enum UnitSignal {
    New,
    Removed,
}

enum SystemdMonitorEvent {
    Signal {
        signal: UnitSignal,
        message: zbus::Message,
    },
    Reconcile(Vec<crate::systemd::Unit>),
}

#[derive(Default)]
struct AppScopeTracker {
    unit_app_ids: HashMap<String, String>,
    app_unit_counts: HashMap<String, usize>,
}

impl AppScopeTracker {
    fn from_units(units: Vec<crate::systemd::Unit>) -> Self {
        let mut tracker = Self::default();
        tracker.rebuild_from_unit_names(units.into_iter().map(|unit| unit.name));
        tracker
    }

    fn reconcile_units(&mut self, units: Vec<crate::systemd::Unit>) -> bool {
        let previous_app_ids = self.app_ids();
        self.rebuild_from_unit_names(units.into_iter().map(|unit| unit.name));
        self.app_ids() != previous_app_ids
    }

    fn rebuild_from_unit_names(&mut self, unit_names: impl IntoIterator<Item = String>) {
        self.unit_app_ids.clear();
        self.app_unit_counts.clear();

        for unit_name in unit_names {
            self.insert_unit(unit_name);
        }
    }

    fn app_ids(&self) -> HashSet<String> {
        self.app_unit_counts.keys().cloned().collect()
    }

    fn insert_unit(&mut self, unit_name: String) -> bool {
        let Some(app_id) = crate::systemd::parse_app_scope_name(&unit_name).map(ToOwned::to_owned)
        else {
            return false;
        };

        if self.unit_app_ids.contains_key(&unit_name) {
            return false;
        }

        let count = self.app_unit_counts.entry(app_id.clone()).or_default();
        let app_started = *count == 0;
        *count += 1;
        self.unit_app_ids.insert(unit_name, app_id);
        app_started
    }

    fn remove_unit(&mut self, unit_name: &str) -> bool {
        let Some(app_id) = self.unit_app_ids.remove(unit_name) else {
            return false;
        };

        let Some(count) = self.app_unit_counts.get_mut(&app_id) else {
            return false;
        };

        *count -= 1;
        if *count == 0 {
            self.app_unit_counts.remove(&app_id);
            true
        } else {
            false
        }
    }
}

async fn emit_running_applications_changed_for_app_scope(
    signal_context: &SignalEmitter<'_>,
    app_scope_tracker: &mut AppScopeTracker,
    signal: UnitSignal,
    message: &zbus::Message,
) -> zbus::Result<()> {
    let Some(unit_name) = crate::background::unit_name_from_systemd_signal(message) else {
        return Ok(());
    };

    let app_set_changed = match signal {
        UnitSignal::New => app_scope_tracker.insert_unit(unit_name),
        UnitSignal::Removed => app_scope_tracker.remove_unit(&unit_name),
    };

    if app_set_changed {
        BackgroundBackend::running_applications_changed(signal_context).await?;
    }

    Ok(())
}
