mod wayland_backend;

use std::{
    collections::HashMap,
    sync::{Arc, LazyLock},
    time::Duration,
};

use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, mpsc, oneshot, watch};
use zbus::{
    interface,
    object_server::SignalEmitter,
    zvariant::{
        Fd, ObjectPath, OwnedObjectPath, OwnedValue, Type,
        as_value::{self, optional},
    },
};

use crate::session::{SESSIONS, SessionType};
use wayland_backend::{ClipboardRequest, ClipboardThread, OwnerState, TransferEvent};

const SIGNAL_QUEUE_DEPTH: usize = 64;
const INIT_TIMEOUT: Duration = Duration::from_secs(5);
const WORKER_REPLY_TIMEOUT: Duration = Duration::from_secs(1);

static CLIPBOARD_SESSION: LazyLock<Arc<Mutex<HashMap<OwnedObjectPath, ClipboardEntry>>>> =
    LazyLock::new(|| Arc::new(Mutex::new(HashMap::new())));

struct ForwarderParts {
    emitter: SignalEmitter<'static>,
    transfer_rx: mpsc::Receiver<TransferEvent>,
    owner_rx: watch::Receiver<Option<OwnerState>>,
}

struct ClipboardEntry {
    thread: ClipboardThread,
    forwarder: Option<ForwarderParts>,
}

#[derive(Debug, Default, Deserialize, Type)]
#[zvariant(signature = "dict")]
struct SetSelectionOptions {
    #[serde(with = "optional", skip_serializing_if = "Option::is_none", default)]
    mime_types: Option<Vec<String>>,
}

#[derive(Debug, Type, Serialize)]
#[zvariant(signature = "dict")]
struct SelectionOwnerChangedOptions {
    #[serde(with = "as_value")]
    mime_types: Vec<String>,
    #[serde(with = "as_value")]
    session_is_owner: bool,
}

pub struct Clipboard;

fn request_failed(message: impl Into<String>) -> zbus::fdo::Error {
    zbus::fdo::Error::Failed(message.into())
}

async fn clipboard_sender(
    session_handle: &ObjectPath<'_>,
) -> zbus::fdo::Result<calloop::channel::SyncSender<ClipboardRequest>> {
    let sessions = CLIPBOARD_SESSION.lock().await;
    sessions
        .get(session_handle)
        .map(|entry| entry.thread.sender.clone())
        .ok_or_else(|| request_failed(format!("no such path: {session_handle}")))
}

// NOTE: spawn = false keeps handlers in receive order (RequestClipboard must land before
// Start reads the flag); in exchange every worker await is bounded by WORKER_REPLY_TIMEOUT.
#[interface(name = "org.freedesktop.impl.portal.Clipboard", spawn = false)]
impl Clipboard {
    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        1
    }

    async fn request_clipboard(
        &self,
        session_handle: ObjectPath<'_>,
        _options: HashMap<String, OwnedValue>,
    ) -> zbus::fdo::Result<()> {
        let mut sessions = SESSIONS.lock().await;
        let Some(session) = sessions
            .iter_mut()
            .find(|session| session.handle_path == session_handle.clone().into())
        else {
            return Err(request_failed(format!("no session {session_handle}")));
        };
        if session.session_type != SessionType::Remote {
            return Err(request_failed(
                "clipboard requires a remote desktop session",
            ));
        }
        session.clipboard_requested = true;
        Ok(())
    }

    async fn set_selection(
        &self,
        session_handle: ObjectPath<'_>,
        options: SetSelectionOptions,
    ) -> zbus::fdo::Result<()> {
        let sender = clipboard_sender(&session_handle).await?;
        sender
            .try_send(ClipboardRequest::SetSelection {
                mime_types: options.mime_types.unwrap_or_default(),
            })
            .map_err(|error| request_failed(format!("request selection failed: {error}")))?;
        Ok(())
    }

    async fn selection_write(
        &self,
        session_handle: ObjectPath<'_>,
        serial: u32,
    ) -> zbus::fdo::Result<Fd<'_>> {
        let sender = clipboard_sender(&session_handle).await?;
        let (reply_tx, reply_rx) = oneshot::channel();
        sender
            .try_send(ClipboardRequest::Write {
                serial,
                sender: reply_tx,
            })
            .map_err(|error| request_failed(format!("request selection failed: {error}")))?;
        // NOTE: a timed-out reply_rx is safe to drop; the late send fails and closes the fd
        let fd = tokio::time::timeout(WORKER_REPLY_TIMEOUT, reply_rx)
            .await
            .map_err(|_| request_failed("clipboard worker did not reply in time"))?
            .map_err(|error| request_failed(format!("clipboard worker stopped: {error}")))?
            .map_err(request_failed)?;
        Ok(Fd::from(fd))
    }

    async fn selection_write_done(
        &self,
        session_handle: ObjectPath<'_>,
        serial: u32,
        success: bool,
    ) -> zbus::fdo::Result<()> {
        let sender = clipboard_sender(&session_handle).await?;
        let (reply_tx, reply_rx) = oneshot::channel();
        sender
            .try_send(ClipboardRequest::WriteDone {
                serial,
                success,
                sender: reply_tx,
            })
            .map_err(|error| request_failed(format!("request selection failed: {error}")))?;
        tokio::time::timeout(WORKER_REPLY_TIMEOUT, reply_rx)
            .await
            .map_err(|_| request_failed("clipboard worker did not reply in time"))?
            .map_err(|error| request_failed(format!("clipboard worker stopped: {error}")))?
            .map_err(request_failed)
    }

    async fn selection_read(
        &self,
        session_handle: ObjectPath<'_>,
        mime_type: &str,
    ) -> zbus::fdo::Result<Fd<'_>> {
        let sender = clipboard_sender(&session_handle).await?;
        let (reply_tx, reply_rx) = oneshot::channel();
        sender
            .try_send(ClipboardRequest::Read {
                mime_type: mime_type.to_owned(),
                sender: reply_tx,
            })
            .map_err(|error| request_failed(format!("request selection failed: {error}")))?;
        let fd = tokio::time::timeout(WORKER_REPLY_TIMEOUT, reply_rx)
            .await
            .map_err(|_| request_failed("clipboard worker did not reply in time"))?
            .map_err(|error| request_failed(format!("clipboard worker stopped: {error}")))?
            .map_err(request_failed)?;
        Ok(Fd::from(fd))
    }

    #[zbus(signal, name = "SelectionOwnerChanged")]
    async fn selection_owner_changed(
        signal_ctx: &SignalEmitter<'_>,
        session_handle: ObjectPath<'_>,
        options: SelectionOwnerChangedOptions,
    ) -> zbus::Result<()>;

    #[zbus(signal, name = "SelectionTransfer")]
    async fn selection_transfer(
        signal_ctx: &SignalEmitter<'_>,
        session_handle: ObjectPath<'_>,
        mime_type: &str,
        serial: u32,
    ) -> zbus::Result<()>;
}

pub(crate) async fn ensure_clipboard_session(
    session_handle: &ObjectPath<'_>,
    connection: zbus::Connection,
) -> bool {
    let key: OwnedObjectPath = session_handle.to_owned().into();
    {
        let sessions_guard = SESSIONS.lock().await;
        if !sessions_guard
            .iter()
            .any(|session| session.handle_path == key)
        {
            return false;
        }
    }

    let (transfer_tx, transfer_rx) = mpsc::channel(SIGNAL_QUEUE_DEPTH);
    let (owner_tx, owner_rx) = watch::channel(None);
    let (ready_tx, ready_rx) = oneshot::channel();
    let thread = match ClipboardThread::spawn(transfer_tx, owner_tx, ready_tx, key.clone()) {
        Ok(thread) => thread,
        Err(error) => {
            tracing::error!(%key, %error, "failed to spawn clipboard worker");
            return false;
        }
    };
    match tokio::time::timeout(INIT_TIMEOUT, ready_rx).await {
        Ok(Ok(Ok(()))) => {}
        Ok(Ok(Err(error))) => {
            tracing::error!(%key, %error, "clipboard init failed");
            return false;
        }
        Ok(Err(error)) => {
            tracing::error!(%key, %error, "clipboard worker died during init");
            return false;
        }
        Err(_) => {
            // A compositor blocked forever in setup leaks this detached worker thread.
            tracing::error!(%key, "clipboard worker init timed out");
            return false;
        }
    }

    let sessions_guard = SESSIONS.lock().await;
    if !sessions_guard
        .iter()
        .any(|session| session.handle_path == key)
    {
        drop(thread);
        return false;
    }
    let mut clipboard_sessions = CLIPBOARD_SESSION.lock().await;
    if thread.exit_rx.has_changed().is_err() {
        tracing::error!(%key, "clipboard worker died before registration");
        return false;
    }

    let emitter = match SignalEmitter::new(&connection, "/org/freedesktop/portal/desktop") {
        Ok(emitter) => emitter.into_owned(),
        Err(error) => {
            tracing::error!(%key, %error, "cannot build clipboard signal emitter");
            return false;
        }
    };
    // a displaced entry (double Start called directly at the impl) is stopped by its Drop
    clipboard_sessions.insert(
        key,
        ClipboardEntry {
            thread,
            forwarder: Some(ForwarderParts {
                emitter,
                transfer_rx,
                owner_rx,
            }),
        },
    );
    true
}

pub(crate) async fn remove_clipboard_session(object_path: ObjectPath<'_>) {
    let mut sessions = CLIPBOARD_SESSION.lock().await;
    if sessions.remove(&object_path).is_some() {
        tracing::info!(%object_path, "clipboard session removal initiated");
    }
}

/// Starts forwarding once the Start reply is on the bus; the first iteration announces
/// the pre-existing owner state. No-op if already spawned or the session is gone.
pub(crate) async fn spawn_clipboard_forwarder(key: &OwnedObjectPath) {
    let mut sessions = CLIPBOARD_SESSION.lock().await;
    let Some(entry) = sessions.get_mut(key) else {
        return;
    };
    let Some(parts) = entry.forwarder.take() else {
        return;
    };
    let exit_rx = entry.thread.exit_rx.clone();
    tokio::spawn(forward_clipboard_signals(
        parts.emitter,
        key.clone(),
        parts.transfer_rx,
        parts.owner_rx,
        exit_rx,
    ));
}

async fn watch_closed(receiver: &mut watch::Receiver<()>) {
    while receiver.changed().await.is_ok() {}
}

async fn forward_clipboard_signals(
    emitter: SignalEmitter<'static>,
    session_handle: OwnedObjectPath,
    mut transfer_rx: mpsc::Receiver<TransferEvent>,
    mut owner_rx: watch::Receiver<Option<OwnerState>>,
    mut exit_rx: watch::Receiver<()>,
) {
    loop {
        if exit_rx.has_changed().is_err() {
            break;
        }
        tokio::select! {
            _ = watch_closed(&mut exit_rx) => break,
            changed = owner_rx.changed() => {
                if changed.is_err() {
                    break;
                }
                let Some(state) = owner_rx.borrow_and_update().clone() else {
                    continue;
                };
                if let Err(error) = Clipboard::selection_owner_changed(
                    &emitter,
                    session_handle.as_ref().clone(),
                    SelectionOwnerChangedOptions {
                        mime_types: state.mime_types,
                        session_is_owner: state.session_is_owner,
                    },
                ).await {
                    // A failed bus emission is dropped; the shared portal connection is broken.
                    tracing::warn!(%error, "failed to emit SelectionOwnerChanged");
                }
            }
            maybe = transfer_rx.recv() => {
                let Some(TransferEvent { mime_type, serial }) = maybe else {
                    break;
                };
                if let Err(error) = Clipboard::selection_transfer(
                    &emitter,
                    session_handle.as_ref().clone(),
                    &mime_type,
                    serial,
                ).await {
                    // The failed signal pins this bounded transfer slot until teardown.
                    tracing::warn!(serial, %error, "failed to emit SelectionTransfer");
                }
            }
        }
    }
}
