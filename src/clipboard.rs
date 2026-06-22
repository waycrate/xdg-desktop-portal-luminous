mod wayland_backend;
use std::{
    collections::HashMap,
    sync::{Arc, LazyLock},
};

use serde::{Deserialize, Serialize};
use zbus::{
    interface,
    object_server::SignalEmitter,
    zvariant::{Fd, ObjectPath, OwnedObjectPath, OwnedValue, Type, Value, as_value},
};

use crate::{
    clipboard::wayland_backend::ClipboardRequest,
    request::RequestInterface,
    session::{Session, SessionType, append_session},
};
use tokio::sync::Mutex;

use wayland_backend::ClipboardThread;

pub static CLIPBOARD_SESSION: LazyLock<Arc<Mutex<HashMap<OwnedObjectPath, ClipboardThread>>>> =
    LazyLock::new(|| Arc::new(Mutex::new(HashMap::new())));

pub async fn append_clipboard_session(object_path: ObjectPath<'_>, thread: ClipboardThread) {
    let mut sessions = CLIPBOARD_SESSION.lock().await;
    if let Some(thread) = sessions.insert(object_path.into(), thread) {
        thread.stop();
    }
}

pub async fn remove_clipboard_session(object_path: ObjectPath<'_>) {
    let mut sessions = CLIPBOARD_SESSION.lock().await;
    let Some(thread) = sessions.remove(&object_path) else {
        return;
    };
    thread.stop();
    tracing::info!("session {} is stopped", object_path);
}

#[derive(Debug, Serialize, Type, Deserialize)]
#[zvariant(signature = "dict")]
struct SelectionOpt {
    #[serde(with = "as_value")]
    mime_types: Vec<String>,
    #[serde(with = "as_value")]
    session_handle: OwnedObjectPath,
    #[serde(flatten)]
    options_rest: HashMap<String, OwnedValue>,
}

#[derive(Debug, Type, Serialize)]
#[zvariant(signature = "dict")]
struct OwnerChanged<'a> {
    #[serde(with = "as_value")]
    mime_types: Vec<&'a str>,
    #[serde(with = "as_value")]
    session_is_owner: bool,
}

pub struct Clipboard;

#[interface(name = "org.freedesktop.impl.portal.Clipboard")]
impl Clipboard {
    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        1
    }

    async fn request_clipboard(
        &self,
        session_handle: ObjectPath<'_>,
        _options: HashMap<&'_ str, Value<'_>>,
        #[zbus(object_server)] server: &zbus::ObjectServer,
    ) -> zbus::fdo::Result<()> {
        tracing::info!("Start clipboard: path :{}", session_handle.as_str(),);
        server
            .at(
                session_handle.clone(),
                RequestInterface {
                    handle_path: session_handle.clone().into(),
                    close_action: None,
                },
            )
            .await?;
        let current_session = Session::new(session_handle.clone(), SessionType::Clipboard);
        append_session(current_session.clone()).await;
        let clipboard_thread = ClipboardThread::new();
        append_clipboard_session(session_handle, clipboard_thread).await;
        Ok(())
    }

    async fn set_selection(
        &self,
        session_handle: ObjectPath<'_>,
        options: SelectionOpt,
    ) -> zbus::fdo::Result<()> {
        let sessions = CLIPBOARD_SESSION.lock().await;
        let Some(thread) = sessions.get(&session_handle) else {
            return Err(zbus::fdo::Error::Failed(format!(
                "no such path: {session_handle}"
            )));
        };
        thread
            .sender
            .send(ClipboardRequest::SetSelection {
                mime_types: options.mime_types,
            })
            .map_err(|e| zbus::fdo::Error::Failed(format!("request selection failed: {e}")))?;
        Ok(())
    }

    async fn selection_write(
        &self,
        session_handle: ObjectPath<'_>,
        serial: u32,
        #[zbus(signal_emitter)] ctx: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<Fd<'_>> {
        let sessions = CLIPBOARD_SESSION.lock().await;
        let Some(thread) = sessions.get(&session_handle) else {
            return Err(zbus::fdo::Error::Failed(format!(
                "no such path: {session_handle}"
            )));
        };
        let (sender, receiver) = tokio::sync::oneshot::channel();
        thread
            .sender
            .send(ClipboardRequest::Write { sender, serial })
            .map_err(|e| zbus::fdo::Error::Failed(format!("request selection failed: {e}")))?;
        let (fd, mime_type) = receiver
            .await
            .map_err(|e| zbus::fdo::Error::Failed(format!("request selection failed: {e}")))?;
        let _ = Self::session_transfer(&ctx, mime_type.as_str(), serial).await;
        Ok(Fd::from(fd))
    }

    async fn selection_write_done(
        &self,
        session_handle: ObjectPath<'_>,
        serial: u32,
        success: bool,
    ) -> zbus::fdo::Result<()> {
        // TODO: what to do here?
        Ok(())
    }

    async fn selection_read(
        &self,
        session_handle: ObjectPath<'_>,
        mime_type: &'_ str,
    ) -> zbus::fdo::Result<Fd<'_>> {
        let sessions = CLIPBOARD_SESSION.lock().await;
        let Some(thread) = sessions.get(&session_handle) else {
            return Err(zbus::fdo::Error::Failed(format!(
                "no such path: {session_handle}"
            )));
        };
        let (sender, receiver) = tokio::sync::oneshot::channel();
        thread
            .sender
            .send(ClipboardRequest::Read {
                sender,
                mime_type: mime_type.to_owned(),
            })
            .map_err(|e| zbus::fdo::Error::Failed(format!("request selection failed: {e}")))?;
        let fd = receiver
            .await
            .map_err(|e| zbus::fdo::Error::Failed(format!("request selection failed: {e}")))?;
        Ok(Fd::from(fd))
    }

    #[zbus(signal)]
    async fn session_owner_changed(
        signal_ctx: &SignalEmitter<'_>,
        session_handle: ObjectPath<'_>,
        options: OwnerChanged<'_>,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn session_transfer(
        signal_ctx: &SignalEmitter<'_>,
        mime_type: &'_ str,
        serial: u32,
    ) -> zbus::Result<()>;
}
