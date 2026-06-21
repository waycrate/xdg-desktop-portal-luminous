mod wayland_backend;
use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use zbus::{
    interface,
    object_server::SignalEmitter,
    zvariant::{Fd, ObjectPath, OwnedObjectPath, OwnedValue, Type, Value, as_value},
};

use crate::{
    request::RequestInterface,
    session::{Session, SessionType, append_session},
};

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
        Ok(())
    }

    async fn set_selection(&self, options: SelectionOpt) -> zbus::fdo::Result<()> {
        todo!()
    }

    async fn selection_write(
        &self,
        session_handle: ObjectPath<'_>,
        serial: u32,
    ) -> zbus::fdo::Result<Fd<'_>> {
        todo!()
    }

    async fn selection_write_done(
        &self,
        session_handle: ObjectPath<'_>,
        serial: u32,
        success: bool,
    ) -> zbus::fdo::Result<()> {
        todo!()
    }

    async fn selection_read(
        &self,
        session_handle: ObjectPath<'_>,
        mime_type: &'_ str,
    ) -> zbus::fdo::Result<Fd<'_>> {
        todo!()
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
