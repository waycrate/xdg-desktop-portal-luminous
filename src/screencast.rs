use std::collections::HashMap;

use zbus::interface;

use zbus::zvariant::{
    ObjectPath, OwnedValue, Type, Value,
    as_value::{self, optional},
};

use enumflags2::BitFlags;

use serde::{Deserialize, Serialize};

use std::sync::Arc;
use std::sync::LazyLock;
use tokio::sync::Mutex;

use crate::PortalResponse;
use crate::pipewirethread::ScreencastThread;
use crate::request::RequestInterface;
use crate::session::{
    CursorMode, PersistMode, SESSIONS, Session, SessionType, SourceType, append_session,
};

use libwaysip::SelectionType;

#[derive(Type, Debug, Default, Serialize, Deserialize)]
/// Specified options for a [`Screencast::create_session`] request.
#[zvariant(signature = "dict")]
struct SessionCreateResult {
    #[serde(with = "as_value")]
    handle_token: String,
}

#[derive(Type, Debug, Default, Serialize, Deserialize)]
/// Specified options for a [`Screencast::select_sources`] request.
#[zvariant(signature = "dict")]
pub struct SelectSourcesOptions {
    /// A string that will be used as the last element of the handle.
    /// What types of content to record.    
    #[serde(with = "optional", skip_serializing_if = "Option::is_none", default)]
    pub types: Option<BitFlags<SourceType>>,
    /// Whether to allow selecting multiple sources.
    #[serde(with = "optional", skip_serializing_if = "Option::is_none", default)]
    pub multiple: Option<bool>,
    /// Determines how the cursor will be drawn in the screen cast stream.
    #[serde(with = "optional", skip_serializing_if = "Option::is_none", default)]
    pub cursor_mode: Option<CursorMode>,
    #[serde(with = "optional", skip_serializing_if = "Option::is_none", default)]
    pub restore_token: Option<String>,
    #[serde(with = "optional", skip_serializing_if = "Option::is_none", default)]
    pub persist_mode: Option<PersistMode>,
}

#[derive(Clone, Serialize, Deserialize, Type, Default, Debug)]
/// A PipeWire stream.
pub struct Stream(u32, StreamProperties);

#[derive(Clone, Default, Type, Debug, Serialize, Deserialize)]
/// The stream properties.
#[zvariant(signature = "dict")]
struct StreamProperties {
    #[serde(with = "optional", skip_serializing_if = "Option::is_none", default)]
    id: Option<String>,
    #[serde(with = "optional", skip_serializing_if = "Option::is_none", default)]
    position: Option<(i32, i32)>,
    #[serde(with = "as_value")]
    size: (i32, i32),
    #[serde(with = "as_value")]
    source_type: SourceType,
}

// TODO: this is copy from ashpd, but the dict is a little different from xdg_desktop_portal
#[derive(Clone, Default, Debug, Type, Serialize, Deserialize)]
#[zvariant(signature = "dict")]
struct StartReturnValue {
    #[serde(with = "as_value")]
    streams: Vec<Stream>,
    #[serde(with = "as_value")]
    persist_mode: u32,
    #[serde(with = "optional", skip_serializing_if = "Option::is_none", default)]
    restore_token: Option<String>,
}

pub struct CastSessionData {
    session_handle: String,
    cast_thread: ScreencastThread,
}
pub static CAST_SESSIONS: LazyLock<Arc<Mutex<Vec<CastSessionData>>>> =
    LazyLock::new(|| Arc::new(Mutex::new(Vec::new())));

pub async fn append_cast_session(session: CastSessionData) {
    let mut sessions = CAST_SESSIONS.lock().await;
    sessions.push(session)
}

pub async fn remove_cast_session(path: &str) {
    let mut sessions = CAST_SESSIONS.lock().await;
    let Some(index) = sessions
        .iter()
        .position(|the_session| the_session.session_handle == path)
    else {
        return;
    };
    sessions[index].cast_thread.stop();
    tracing::info!("session {} is stopped", sessions[index].session_handle);
    sessions.remove(index);
}

pub struct ScreenCastBackend;

#[interface(name = "org.freedesktop.impl.portal.ScreenCast")]
impl ScreenCastBackend {
    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        4
    }

    #[zbus(property)]
    fn available_cursor_modes(&self) -> u32 {
        (CursorMode::Hidden | CursorMode::Embedded).bits()
    }

    #[zbus(property)]
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
    ) -> zbus::fdo::Result<PortalResponse<SessionCreateResult>> {
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
        let current_session = Session::new(session_handle.clone(), SessionType::ScreenCast);
        append_session(current_session.clone()).await;
        server.at(session_handle.clone(), current_session).await?;
        Ok(PortalResponse::Success(SessionCreateResult {
            handle_token: session_handle.to_string(),
        }))
    }

    async fn select_sources(
        &self,
        _request_handle: ObjectPath<'_>,
        session_handle: ObjectPath<'_>,
        _app_id: String,
        options: SelectSourcesOptions,
    ) -> zbus::fdo::Result<PortalResponse<HashMap<String, OwnedValue>>> {
        let mut locked_sessions = SESSIONS.lock().await;
        let Some(index) = locked_sessions
            .iter()
            .position(|this_session| this_session.handle_path == session_handle.clone().into())
        else {
            tracing::warn!("No session is created or it is removed");
            return Ok(PortalResponse::Other);
        };
        locked_sessions[index].set_screencast_options(options);
        Ok(PortalResponse::Success(HashMap::new()))
    }

    async fn start(
        &self,
        _request_handle: ObjectPath<'_>,
        session_handle: ObjectPath<'_>,
        _app_id: String,
        _parent_window: String,
        _options: HashMap<String, Value<'_>>,
    ) -> zbus::fdo::Result<PortalResponse<StartReturnValue>> {
        let cast_sessions = CAST_SESSIONS.lock().await;
        if let Some(session) = cast_sessions
            .iter()
            .find(|session| session.session_handle == session_handle.to_string())
        {
            return Ok(PortalResponse::Success(StartReturnValue {
                streams: vec![Stream(
                    session.cast_thread.node_id(),
                    StreamProperties::default(),
                )],
                ..Default::default()
            }));
        }
        drop(cast_sessions);

        let locked_sessions = SESSIONS.lock().await;
        let Some(index) = locked_sessions
            .iter()
            .position(|this_session| this_session.handle_path == session_handle.clone().into())
        else {
            tracing::warn!("No session is created or it is removed");
            return Ok(PortalResponse::Other);
        };

        let current_session = locked_sessions[index].clone();
        if current_session.session_type != SessionType::ScreenCast {
            return Ok(PortalResponse::Other);
        }
        drop(locked_sessions);

        let show_cursor = current_session.cursor_mode.show_cursor();
        let connection = libwayshot::WayshotConnection::new().unwrap();

        let info = match libwaysip::get_area(
            Some(libwaysip::WaysipConnection {
                connection: &connection.conn,
                globals: &connection.globals,
            }),
            SelectionType::Screen,
        ) {
            Ok(Some(info)) => info,
            Ok(None) => return Err(zbus::Error::Failure("You cancel it".to_string()).into()),
            Err(e) => return Err(zbus::Error::Failure(format!("wayland error, {e}")).into()),
        };

        use libwaysip::Size;
        let screen_info = info.screen_info;

        let Size { width, height } = screen_info.get_wloutput_size();
        let output = screen_info.wl_output;
        let cast_thread = ScreencastThread::start_cast(
            show_cursor,
            width as u32,
            height as u32,
            None,
            output,
            connection,
        )
        .await
        .map_err(|e| zbus::Error::Failure(format!("cannot start pipewire stream, error: {e}")))?;

        let node_id = cast_thread.node_id();

        append_cast_session(CastSessionData {
            session_handle: session_handle.to_string(),
            cast_thread,
        })
        .await;

        Ok(PortalResponse::Success(StartReturnValue {
            streams: vec![Stream(
                node_id,
                StreamProperties {
                    size: (width, height),
                    source_type: SourceType::Monitor,
                    ..Default::default()
                },
            )],
            ..Default::default()
        }))
    }
}
