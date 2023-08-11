use std::collections::HashMap;

use zbus::dbus_interface;

use zbus::zvariant::{DeserializeDict, ObjectPath, OwnedValue, SerializeDict, Type, Value};

use enumflags2::BitFlags;

use serde::{Deserialize, Serialize};

use crate::request::RequestInterface;
use crate::session::{append_session, CursorMode, PersistMode, Session, SourceType, SESSIONS};

#[derive(SerializeDict, DeserializeDict, Type, Debug, Default)]
/// Specified options for a [`Screencast::create_session`] request.
#[zvariant(signature = "dict")]
struct SessionCreateResult {
    handle_token: String,
}

#[derive(SerializeDict, DeserializeDict, Type, Debug, Default)]
/// Specified options for a [`Screencast::select_sources`] request.
#[zvariant(signature = "dict")]
pub struct SelectSourcesOptions {
    /// A string that will be used as the last element of the handle.
    /// What types of content to record.
    pub types: Option<BitFlags<SourceType>>,
    /// Whether to allow selecting multiple sources.
    pub multiple: Option<bool>,
    /// Determines how the cursor will be drawn in the screen cast stream.
    pub cursor_mode: Option<CursorMode>,
    pub restore_token: Option<String>,
    pub persist_mode: Option<PersistMode>,
}

#[derive(Clone, Serialize, Deserialize, Type, Default, Debug)]
/// A PipeWire stream.
pub struct Stream(u32, StreamProperties);

#[derive(Clone, SerializeDict, DeserializeDict, Default, Type, Debug)]
/// The stream properties.
#[zvariant(signature = "dict")]
struct StreamProperties {
    id: Option<String>,
    position: Option<(i32, i32)>,
    size: Option<(i32, i32)>,
    source_type: Option<SourceType>,
}

// TODO: this is copy from ashpd, but the dict is a little different from xdg_desktop_portal
#[derive(Clone, SerializeDict, DeserializeDict, Default, Debug, Type)]
#[zvariant(signature = "dict")]
struct StartReturnValue {
    streams: Vec<Stream>,
    persist_mode: u32,
    restore_token: Option<String>,
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
        (CursorMode::Hidden | CursorMode::Embedded).bits()
    }

    #[dbus_interface(property)]
    fn available_source_types(&self) -> u32 {
        BitFlags::from_flag(SourceType::Monitor).bits()
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
        let current_session = Session::new(session_handle.clone());
        append_session(current_session.clone()).await;
        server.at(session_handle.clone(), current_session).await?;
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
        session_handle: ObjectPath<'_>,
        _app_id: String,
        options: SelectSourcesOptions,
    ) -> zbus::fdo::Result<(u32, HashMap<String, OwnedValue>)> {
        let mut locked_sessions = SESSIONS.lock().await;
        let Some(index) = locked_sessions.iter().position(|this_session| this_session.handle_path == session_handle.clone().into()) else {
            tracing::warn!("No session is created or it is removed");
            return Ok((2, HashMap::new()));
        };
        locked_sessions[index].set_options(options);
        Ok((0, HashMap::new()))
    }

    async fn start(
        &self,
        _request_handle: ObjectPath<'_>,
        session_handle: ObjectPath<'_>,
        _app_id: String,
        _parent_window: String,
        _options: HashMap<String, Value<'_>>,
    ) -> zbus::fdo::Result<(u32, StartReturnValue)> {
        let locked_sessions = SESSIONS.lock().await;
        let Some(index) = locked_sessions.iter().position(|this_session| this_session.handle_path == session_handle.clone().into()) else {
            tracing::warn!("No session is created or it is removed");
            return Ok((2, StartReturnValue::default()));
        };
        let current_session = locked_sessions[index].clone();
        drop(locked_sessions);
        let show_cursor = current_session.cursor_mode.show_cursor();
        println!("{:?}", current_session);
        println!("{show_cursor}");
        Ok((0, StartReturnValue::default()))
    }
}
