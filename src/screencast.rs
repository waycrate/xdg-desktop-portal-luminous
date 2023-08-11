use std::collections::HashMap;

use zbus::dbus_interface;

use zbus::zvariant::{DeserializeDict, ObjectPath, OwnedValue, SerializeDict, Type, Value};

use enumflags2::BitFlags;

use crate::request::RequestInterface;
use crate::session::{CursorMode, PersistMode, Session, SourceType};

#[derive(SerializeDict, DeserializeDict, Type, Debug, Default)]
/// Specified options for a [`Screencast::create_session`] request.
#[zvariant(signature = "dict")]
struct SessionCreateResult {
    handle_token: String,
}

#[derive(SerializeDict, DeserializeDict, Type, Debug, Default)]
/// Specified options for a [`Screencast::select_sources`] request.
#[zvariant(signature = "dict")]
struct SelectSourcesOptions {
    /// A string that will be used as the last element of the handle.
    /// What types of content to record.
    types: Option<BitFlags<SourceType>>,
    /// Whether to allow selecting multiple sources.
    multiple: Option<bool>,
    /// Determines how the cursor will be drawn in the screen cast stream.
    cursor_mode: Option<CursorMode>,
    restore_token: Option<String>,
    persist_mode: Option<PersistMode>,
}

pub struct ScreenCast;

#[dbus_interface(name = "org.freedesktop.impl.portal.ScreenCast")]
impl ScreenCast {
    #[dbus_interface(property, name = "version")]
    fn version(&self) -> u32 {
        4
    }

    #[dbus_interface(property)]
    fn available_cursor_modes(&self) -> u32 {
        (CursorMode::Hidden | CursorMode::Embedded).bits().into()
    }

    #[dbus_interface(property)]
    fn available_source_types(&self) -> u32 {
        (SourceType::Monitor | SourceType::Monitor).bits().into()
    }

    async fn create_session(
        &self,
        request_handle: ObjectPath<'_>,
        session_handle: ObjectPath<'_>,
        app_id: String,
        _options: HashMap<String, Value<'_>>,
        #[zbus(object_server)] server: &zbus::ObjectServer,
    ) -> zbus::fdo::Result<(u32, SessionCreateResult)> {
        tracing::info!(
            "Start shot: path :{}, appid: {}",
            request_handle.as_str(),
            app_id
        );
        server
            .at(
                request_handle.clone(),
                RequestInterface {
                    handle_path: request_handle.clone().into(),
                },
            )
            .await?;
        server
            .at(session_handle.clone(), Session::default())
            .await?;
        Ok((
            0,
            SessionCreateResult {
                handle_token: session_handle.to_string(),
            },
        ))
    }

    async fn select_sources(
        &self,
        _request_handle: ObjectPath<'_>,
        _session_handle: ObjectPath<'_>,
        _app_id: String,
        _options: SelectSourcesOptions,
    ) -> zbus::fdo::Result<(u32, HashMap<String, OwnedValue>)> {
        // TODO: do nothing here now
        Ok((0, HashMap::new()))
    }

    async fn start(
        &self,
        request_handle: ObjectPath<'_>,
        session_handle: ObjectPath<'_>,
        app_id: String,
        _options: HashMap<String, Value<'_>>,
    ) -> zbus::fdo::Result<(u32, HashMap<String, OwnedValue>)> {
        println!("ssssss");
        println!("{:?}", _options);
        Ok((0, HashMap::new()))
    }
}
