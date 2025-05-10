mod dispatch;
mod remote_thread;
mod state;

use libwaysip::SelectionType;
use remote_thread::RemoteControl;

use std::collections::HashMap;

use enumflags2::BitFlags;
use zbus::interface;

use zbus::zvariant::{
    ObjectPath, OwnedValue, Type, Value,
    as_value::{self, optional},
};

use serde::{Deserialize, Serialize};

use std::sync::{Arc, LazyLock};
use tokio::sync::Mutex;

use crate::pipewirethread::ScreencastThread;
use crate::request::RequestInterface;
use crate::session::{
    DeviceType, PersistMode, SESSIONS, Session, SessionType, SourceType, append_session,
};

use crate::PortalResponse;

use self::remote_thread::KeyOrPointerRequest;

#[derive(Type, Debug, Default, Serialize, Deserialize)]
/// Specified options for a [`Screencast::create_session`] request.
#[zvariant(signature = "dict")]
struct SessionCreateResult {
    #[serde(with = "as_value")]
    handle_token: String,
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
struct RemoteStartReturnValue {
    #[serde(with = "as_value")]
    streams: Vec<Stream>,
    #[serde(with = "as_value")]
    devices: BitFlags<DeviceType>,
    #[serde(with = "as_value")]
    clipboard_enabled: bool,
    #[serde(with = "as_value")]
    screen_share_enabled: bool,
}

#[derive(Type, Debug, Default, Deserialize, Serialize)]
/// Specified options for a [`RemoteDesktop::select_devices`] request.
#[zvariant(signature = "dict")]
pub struct SelectDevicesOptions {
    /// A string that will be used as the last element of the handle.
    /// The device types to request remote controlling of. Default is all.
    #[serde(with = "optional", skip_serializing_if = "Option::is_none", default)]
    pub types: Option<BitFlags<DeviceType>>,
    #[serde(with = "optional", skip_serializing_if = "Option::is_none", default)]
    pub restore_token: Option<String>,
    #[serde(with = "optional", skip_serializing_if = "Option::is_none", default)]
    pub persist_mode: Option<PersistMode>,
}

pub struct RemoteSessionData {
    session_handle: String,
    cast_thread: ScreencastThread,
    remote_control: RemoteControl,
}

pub static REMOTE_SESSIONS: LazyLock<Arc<Mutex<Vec<RemoteSessionData>>>> =
    LazyLock::new(|| Arc::new(Mutex::new(Vec::new())));

pub async fn append_remote_session(session: RemoteSessionData) {
    let mut sessions = REMOTE_SESSIONS.lock().await;
    sessions.push(session)
}

pub async fn remove_remote_session(path: &str) {
    let mut sessions = REMOTE_SESSIONS.lock().await;
    let Some(index) = sessions
        .iter()
        .position(|the_session| the_session.session_handle == path)
    else {
        return;
    };
    sessions[index].cast_thread.stop();
    sessions[index].remote_control.stop();
    tracing::info!("session {} is stopped", sessions[index].session_handle);
    sessions.remove(index);
}

pub struct RemoteDesktopBackend;

#[interface(name = "org.freedesktop.impl.portal.RemoteDesktop")]
impl RemoteDesktopBackend {
    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        2
    }

    #[zbus(property)]
    fn available_device_types(&self) -> u32 {
        (DeviceType::Keyboard | DeviceType::Pointer).bits()
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
        let current_session = Session::new(session_handle.clone(), SessionType::Remote);
        append_session(current_session.clone()).await;
        server.at(session_handle.clone(), current_session).await?;
        Ok(PortalResponse::Success(SessionCreateResult {
            handle_token: session_handle.to_string(),
        }))
    }

    async fn select_devices(
        &self,
        _request_handle: ObjectPath<'_>,
        session_handle: ObjectPath<'_>,
        _app_id: String,
        options: SelectDevicesOptions,
    ) -> zbus::fdo::Result<PortalResponse<HashMap<String, OwnedValue>>> {
        let mut locked_sessions = SESSIONS.lock().await;
        let Some(index) = locked_sessions
            .iter()
            .position(|this_session| this_session.handle_path == session_handle.clone().into())
        else {
            tracing::warn!("No session is created or it is removed");
            return Ok(PortalResponse::Other);
        };
        if locked_sessions[index].session_type != SessionType::Remote {
            return Ok(PortalResponse::Other);
        }
        locked_sessions[index].set_remote_options(options);
        Ok(PortalResponse::Success(HashMap::new()))
    }

    async fn start(
        &self,
        _request_handle: ObjectPath<'_>,
        session_handle: ObjectPath<'_>,
        _app_id: String,
        _parent_window: String,
        _options: HashMap<String, Value<'_>>,
    ) -> zbus::fdo::Result<PortalResponse<RemoteStartReturnValue>> {
        let locked_sessions = SESSIONS.lock().await;
        let Some(index) = locked_sessions
            .iter()
            .position(|this_session| this_session.handle_path == session_handle.clone().into())
        else {
            tracing::warn!("No session is created or it is removed");
            return Ok(PortalResponse::Other);
        };

        let current_session = locked_sessions[index].clone();
        if current_session.session_type != SessionType::Remote {
            return Ok(PortalResponse::Other);
        }
        let device_type = current_session.device_type;
        drop(locked_sessions);

        let remote_sessions = REMOTE_SESSIONS.lock().await;
        if let Some(session) = remote_sessions
            .iter()
            .find(|session| session.session_handle == session_handle.to_string())
        {
            return Ok(PortalResponse::Success(RemoteStartReturnValue {
                streams: vec![Stream(
                    session.cast_thread.node_id(),
                    StreamProperties::default(),
                )],
                devices: device_type,
                ..Default::default()
            }));
        }
        drop(remote_sessions);

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

        tracing::info!("{width}, {height}");
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

        let remote_control = RemoteControl::init();
        let node_id = cast_thread.node_id();

        append_remote_session(RemoteSessionData {
            session_handle: session_handle.to_string(),
            cast_thread,
            remote_control,
        })
        .await;

        Ok(PortalResponse::Success(RemoteStartReturnValue {
            streams: vec![Stream(
                node_id,
                StreamProperties {
                    size: (width, height),
                    source_type: SourceType::Monitor,
                    ..Default::default()
                },
            )],
            devices: device_type,
            screen_share_enabled: true,
            ..Default::default()
        }))
    }

    // keyboard and else
    async fn notify_pointer_motion(
        &self,
        session_handle: ObjectPath<'_>,
        _options: HashMap<String, Value<'_>>,
        dx: f64,
        dy: f64,
    ) -> zbus::fdo::Result<()> {
        let remote_sessions = REMOTE_SESSIONS.lock().await;
        let Some(session) = remote_sessions
            .iter()
            .find(|session| session.session_handle == session_handle.to_string())
        else {
            return Ok(());
        };
        let remote_control = &session.remote_control;
        remote_control
            .sender
            .send(KeyOrPointerRequest::PointerMotion { dx, dy })
            .map_err(|_| zbus::Error::Failure("Send failed".to_string()))?;
        Ok(())
    }

    async fn notify_pointer_motion_absolute(
        &self,
        session_handle: ObjectPath<'_>,
        _options: HashMap<String, Value<'_>>,
        _steam: u32,
        x: f64,
        y: f64,
    ) -> zbus::fdo::Result<()> {
        let remote_sessions = REMOTE_SESSIONS.lock().await;
        let Some(session) = remote_sessions
            .iter()
            .find(|session| session.session_handle == session_handle.to_string())
        else {
            return Ok(());
        };
        let remote_control = &session.remote_control;
        remote_control
            .sender
            .send(KeyOrPointerRequest::PointerMotionAbsolute {
                x,
                y,
                x_extent: 2000,
                y_extent: 2000,
            })
            .map_err(|_| zbus::Error::Failure("Send failed".to_string()))?;
        Ok(())
    }

    async fn notify_pointer_button(
        &self,
        session_handle: ObjectPath<'_>,
        _options: HashMap<String, Value<'_>>,
        button: i32,
        state: u32,
    ) -> zbus::fdo::Result<()> {
        let remote_sessions = REMOTE_SESSIONS.lock().await;
        let Some(session) = remote_sessions
            .iter()
            .find(|session| session.session_handle == session_handle.to_string())
        else {
            return Ok(());
        };
        let remote_control = &session.remote_control;
        remote_control
            .sender
            .send(KeyOrPointerRequest::PointerButton { button, state })
            .map_err(|_| zbus::Error::Failure("Send failed".to_string()))?;
        Ok(())
    }

    async fn notify_pointer_axis(
        &self,
        session_handle: ObjectPath<'_>,
        _options: HashMap<String, Value<'_>>,
        dx: f64,
        dy: f64,
    ) -> zbus::fdo::Result<()> {
        let remote_sessions = REMOTE_SESSIONS.lock().await;
        let Some(session) = remote_sessions
            .iter()
            .find(|session| session.session_handle == session_handle.to_string())
        else {
            return Ok(());
        };
        let remote_control = &session.remote_control;
        remote_control
            .sender
            .send(KeyOrPointerRequest::PointerAxis { dx, dy })
            .map_err(|_| zbus::Error::Failure("Send failed".to_string()))?;
        Ok(())
    }

    async fn notify_pointer_axis_discrate(
        &self,
        session_handle: ObjectPath<'_>,
        _options: HashMap<String, Value<'_>>,
        axis: u32,
        steps: i32,
    ) -> zbus::fdo::Result<()> {
        let remote_sessions = REMOTE_SESSIONS.lock().await;
        let Some(session) = remote_sessions
            .iter()
            .find(|session| session.session_handle == session_handle.to_string())
        else {
            return Ok(());
        };
        let remote_control = &session.remote_control;
        remote_control
            .sender
            .send(KeyOrPointerRequest::PointerAxisDiscrate { axis, steps })
            .map_err(|_| zbus::Error::Failure("Send failed".to_string()))?;
        Ok(())
    }

    async fn notify_keyboard_keycode(
        &self,
        session_handle: ObjectPath<'_>,
        _options: HashMap<String, Value<'_>>,
        keycode: i32,
        state: u32,
    ) -> zbus::fdo::Result<()> {
        let remote_sessions = REMOTE_SESSIONS.lock().await;
        let Some(session) = remote_sessions
            .iter()
            .find(|session| session.session_handle == session_handle.to_string())
        else {
            return Ok(());
        };
        let remote_control = &session.remote_control;
        remote_control
            .sender
            .send(KeyOrPointerRequest::KeyboardKeycode { keycode, state })
            .map_err(|_| zbus::Error::Failure("Send failed".to_string()))?;
        Ok(())
    }

    async fn notify_keyboard_keysym(
        &self,
        session_handle: ObjectPath<'_>,
        _options: HashMap<String, Value<'_>>,
        keysym: i32,
        state: u32,
    ) -> zbus::fdo::Result<()> {
        let remote_sessions = REMOTE_SESSIONS.lock().await;
        let Some(session) = remote_sessions
            .iter()
            .find(|session| session.session_handle == session_handle.to_string())
        else {
            return Ok(());
        };
        let remote_control = &session.remote_control;
        remote_control
            .sender
            .send(KeyOrPointerRequest::KeyboardKeysym { keysym, state })
            .map_err(|_| zbus::Error::Failure("Send failed".to_string()))?;
        Ok(())
    }
}
