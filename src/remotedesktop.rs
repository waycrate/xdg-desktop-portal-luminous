mod dispatch;
mod remote_thread;
mod state;

use remote_thread::RemoteControl;

use std::collections::HashMap;

use enumflags2::BitFlags;
use zbus::dbus_interface;

use zbus::zvariant::{DeserializeDict, ObjectPath, OwnedValue, SerializeDict, Type, Value};

use serde::{Deserialize, Serialize};

use once_cell::sync::Lazy;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::pipewirethread::ScreencastThread;
use crate::request::RequestInterface;
use crate::session::{
    append_session, DeviceType, PersistMode, Session, SessionType, SourceType, SESSIONS,
};

use crate::PortalResponse;

use self::remote_thread::KeyOrPointerRequest;

#[derive(SerializeDict, DeserializeDict, Type, Debug, Default)]
/// Specified options for a [`Screencast::create_session`] request.
#[zvariant(signature = "dict")]
struct SessionCreateResult {
    handle_token: String,
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
struct RemoteStartReturnValue {
    streams: Vec<Stream>,
    devices: BitFlags<DeviceType>,
    clipboard_enabled: bool,
}

#[derive(SerializeDict, DeserializeDict, Type, Debug, Default)]
/// Specified options for a [`RemoteDesktop::select_devices`] request.
#[zvariant(signature = "dict")]
pub struct SelectDevicesOptions {
    /// A string that will be used as the last element of the handle.
    /// The device types to request remote controlling of. Default is all.
    pub types: Option<BitFlags<DeviceType>>,
    pub restore_token: Option<String>,
    pub persist_mode: Option<PersistMode>,
}

pub type RemoteSessionData = (String, ScreencastThread, RemoteControl);
pub static REMOTE_SESSIONS: Lazy<Arc<Mutex<Vec<RemoteSessionData>>>> =
    Lazy::new(|| Arc::new(Mutex::new(Vec::new())));

pub async fn append_remote_session(session: RemoteSessionData) {
    let mut sessions = REMOTE_SESSIONS.lock().await;
    sessions.push(session)
}

pub async fn remove_remote_session(path: &str) {
    let mut sessions = REMOTE_SESSIONS.lock().await;
    let Some(index) = sessions
        .iter()
        .position(|the_session| the_session.0 == path)
    else {
        return;
    };
    sessions[index].1.stop();
    sessions[index].2.stop();
    tracing::info!("session {} is stopped", sessions[index].0);
    sessions.remove(index);
}

pub struct RemoteDesktopBackend;

#[dbus_interface(name = "org.freedesktop.impl.portal.RemoteDesktop")]
impl RemoteDesktopBackend {
    #[dbus_interface(property, name = "version")]
    fn version(&self) -> u32 {
        2
    }

    #[dbus_interface(property)]
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
        let remote_sessions = REMOTE_SESSIONS.lock().await;
        if let Some(session) = remote_sessions
            .iter()
            .find(|session| session.0 == session_handle.to_string())
        {
            return Ok(PortalResponse::Success(RemoteStartReturnValue {
                streams: vec![Stream(session.1.node_id(), StreamProperties::default())],
                ..Default::default()
            }));
        }
        drop(remote_sessions);

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

        // TODO: use slurp now
        let show_cursor = current_session.cursor_mode.show_cursor();
        let connection = libwayshot::WayshotConnection::new().unwrap();
        let outputs = connection.get_all_outputs();
        let slurp = std::process::Command::new("slurp")
            .arg("-o")
            .output()
            .map_err(|_| zbus::Error::Failure("Cannot find slurp".to_string()))?
            .stdout;
        let output = String::from_utf8_lossy(&slurp);
        let output = output
            .split(' ')
            .next()
            .ok_or(zbus::Error::Failure("Not get slurp area".to_string()))?;

        let point: Vec<&str> = output.split(',').collect();
        let x: i32 = point[0]
            .parse()
            .map_err(|_| zbus::Error::Failure("X is not correct".to_string()))?;
        let y: i32 = point[1]
            .parse()
            .map_err(|_| zbus::Error::Failure("Y is not correct".to_string()))?;

        let Some(output) = outputs
            .iter()
            .find(|output| output.dimensions.x == x && output.dimensions.y == y)
        else {
            return Ok(PortalResponse::Other);
        };

        let cast_thread = ScreencastThread::start_cast(
            show_cursor,
            output.mode.width as u32,
            output.mode.height as u32,
            None,
            output.wl_output.clone(),
        )
        .await
        .map_err(|e| zbus::Error::Failure(format!("cannot start pipewire stream, error: {e}")))?;

        let remote_control = RemoteControl::init();
        let node_id = cast_thread.node_id();

        append_remote_session((session_handle.to_string(), cast_thread, remote_control)).await;

        Ok(PortalResponse::Success(RemoteStartReturnValue {
            streams: vec![Stream(node_id, StreamProperties::default())],
            devices: device_type,
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
            .find(|session| session.0 == session_handle.to_string())
        else {
            return Ok(());
        };
        let remote_control = &session.2;
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
            .find(|session| session.0 == session_handle.to_string())
        else {
            return Ok(());
        };
        let remote_control = &session.2;
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
            .find(|session| session.0 == session_handle.to_string())
        else {
            return Ok(());
        };
        let remote_control = &session.2;
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
            .find(|session| session.0 == session_handle.to_string())
        else {
            return Ok(());
        };
        let remote_control = &session.2;
        remote_control
            .sender
            .send(KeyOrPointerRequest::PointerAxis { dx, dy })
            .map_err(|_| zbus::Error::Failure("Send failed".to_string()))?;
        Ok(())
    }

    async fn notify_pointer_axix_discrate(
        &self,
        session_handle: ObjectPath<'_>,
        _options: HashMap<String, Value<'_>>,
        axis: u32,
        steps: i32,
    ) -> zbus::fdo::Result<()> {
        let remote_sessions = REMOTE_SESSIONS.lock().await;
        let Some(session) = remote_sessions
            .iter()
            .find(|session| session.0 == session_handle.to_string())
        else {
            return Ok(());
        };
        let remote_control = &session.2;
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
            .find(|session| session.0 == session_handle.to_string())
        else {
            return Ok(());
        };
        let remote_control = &session.2;
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
            .find(|session| session.0 == session_handle.to_string())
        else {
            return Ok(());
        };
        let remote_control = &session.2;
        remote_control
            .sender
            .send(KeyOrPointerRequest::KeyboardKeysym { keysym, state })
            .map_err(|_| zbus::Error::Failure("Send failed".to_string()))?;
        Ok(())
    }
}
