mod dispatch;
mod eis_server;
mod remote_thread;
mod state;

use libwayshot::WayshotConnection;
use libwaysip::{SelectionType, WaySip};
use remote_thread::RemoteControl;
use stream_message::SERVER_SOCK;
use wayland_client::protocol::wl_output;

use std::collections::HashMap;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, LazyLock, Mutex as StdMutex};

use calloop::channel::Sender;
use enumflags2::BitFlags;
use reis::eis;
use rustix::fd::AsFd;
use rustix::io;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use zbus::interface;
use zbus::zvariant::{
    Fd, ObjectPath, OwnedValue, Type, Value,
    as_value::{self, optional},
};

use crate::PortalResponse;
use crate::pipewirethread::CastTarget;
use crate::pipewirethread::ScreencastThread;
use crate::request::RequestInterface;
use crate::session::{
    DeviceType, PersistMode, SESSIONS, Session, SessionType, SourceType, append_session,
};
use crate::utils::get_selection_from_socket;

use self::eis_server::{EisServerMsg, InputEvent};
use self::remote_thread::InputRequest;

type EisServerSender = Sender<EisServerMsg>;
type InputEventReceiver = Arc<StdMutex<Receiver<InputEvent>>>;

static EIS_SERVER: LazyLock<(EisServerSender, InputEventReceiver)> = LazyLock::new(|| {
    let (tx, rx) = eis_server::start();
    (tx, Arc::new(StdMutex::new(rx)))
});

pub fn get_input_receiver() -> InputEventReceiver {
    EIS_SERVER.1.clone()
}

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
    cast_thread: Option<ScreencastThread>,
    remote_control: RemoteControl,
}

impl RemoteSessionData {
    fn stop(&self) {
        self.remote_control.stop();
        if let Some(cast_thread) = &self.cast_thread {
            cast_thread.stop();
        }
    }

    fn streams(&self) -> Vec<Stream> {
        let Some(cast_thread) = &self.cast_thread else {
            return vec![];
        };
        vec![Stream(cast_thread.node_id(), StreamProperties::default())]
    }
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
    sessions[index].stop();
    tracing::info!("session {} is stopped", sessions[index].session_handle);
    sessions.remove(index);
}

async fn notify_input_event(
    session_handle: ObjectPath<'_>,
    event: InputRequest,
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
        .send(event)
        .map_err(|_| zbus::Error::Failure("Send failed".to_string()))?;
    Ok(())
}

pub async fn handle_input_event(event: InputEvent) {
    let (session_handle, request) = match event {
        InputEvent::PointerMotion {
            session_handle,
            dx,
            dy,
        } => (session_handle, InputRequest::PointerMotion { dx, dy }),
        InputEvent::PointerMotionAbsolute {
            session_handle,
            x,
            y,
        } => (session_handle, InputRequest::PointerMotionAbsolute { x, y }),
        InputEvent::PointerButton {
            session_handle,
            button,
            state,
        } => (
            session_handle,
            InputRequest::PointerButton { button, state },
        ),
        InputEvent::PointerAxis {
            session_handle,
            dx,
            dy,
        } => (session_handle, InputRequest::PointerAxis { dx, dy }),
        InputEvent::PointerAxisDiscrate {
            session_handle,
            axis,
            steps,
        } => (
            session_handle,
            InputRequest::PointerAxisDiscrate { axis, steps },
        ),
        InputEvent::KeyboardKeycode {
            session_handle,
            keycode,
            state,
        } => (
            session_handle,
            InputRequest::KeyboardKeycode { keycode, state },
        ),
        InputEvent::TouchDown {
            session_handle,
            slot,
            x,
            y,
        } => (session_handle, InputRequest::TouchDown { slot, x, y }),
        InputEvent::TouchMotion {
            session_handle,
            slot,
            x,
            y,
        } => (session_handle, InputRequest::TouchMotion { slot, x, y }),
        InputEvent::TouchUp {
            session_handle,
            slot,
        } => (session_handle, InputRequest::TouchUp { slot }),
    };

    if let Ok(path) = ObjectPath::try_from(session_handle) {
        let _ = notify_input_event(path, request).await;
    }
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
                streams: session.streams(),
                devices: device_type,
                ..Default::default()
            }));
        }
        drop(remote_sessions);

        let screen_share_enabled = current_session.screen_share_enabled;
        let mut streams = vec![];
        let mut cast_thread = None;
        let connection = libwayshot::WayshotConnection::new().unwrap();
        let RemoteInfo {
            width,
            height,
            x,
            y,
            wl_output,
        } = get_monitor_info_from_socket(&connection)?;
        if screen_share_enabled {
            let show_cursor = current_session.cursor_mode.show_cursor();

            let output = wl_output;

            let cast_thread_target = ScreencastThread::start_cast(
                show_cursor,
                None,
                CastTarget::Screen(output),
                connection,
            )
            .await
            .map_err(|e| {
                zbus::Error::Failure(format!("cannot start pipewire stream, error: {e}"))
            })?;

            let node_id = cast_thread_target.node_id();
            streams.push(Stream(
                node_id,
                StreamProperties {
                    size: (width, height),
                    source_type: SourceType::Monitor,
                    ..Default::default()
                },
            ));
            cast_thread = Some(cast_thread_target);
        }
        let remote_control = RemoteControl::init(x as u32, y as u32, width as u32, height as u32);

        append_remote_session(RemoteSessionData {
            session_handle: session_handle.to_string(),
            cast_thread,
            remote_control,
        })
        .await;
        Ok(PortalResponse::Success(RemoteStartReturnValue {
            streams,
            devices: device_type,
            screen_share_enabled,
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
        notify_input_event(session_handle, InputRequest::PointerMotion { dx, dy }).await
    }

    async fn notify_pointer_motion_absolute(
        &self,
        session_handle: ObjectPath<'_>,
        _options: HashMap<String, Value<'_>>,
        _steam: u32,
        x: f64,
        y: f64,
    ) -> zbus::fdo::Result<()> {
        notify_input_event(session_handle, InputRequest::PointerMotionAbsolute { x, y }).await
    }

    async fn notify_pointer_button(
        &self,
        session_handle: ObjectPath<'_>,
        _options: HashMap<String, Value<'_>>,
        button: i32,
        state: u32,
    ) -> zbus::fdo::Result<()> {
        notify_input_event(
            session_handle,
            InputRequest::PointerButton { button, state },
        )
        .await
    }

    async fn notify_pointer_axis(
        &self,
        session_handle: ObjectPath<'_>,
        _options: HashMap<String, Value<'_>>,
        dx: f64,
        dy: f64,
    ) -> zbus::fdo::Result<()> {
        notify_input_event(session_handle, InputRequest::PointerAxis { dx, dy }).await
    }

    async fn notify_pointer_axis_discrate(
        &self,
        session_handle: ObjectPath<'_>,
        _options: HashMap<String, Value<'_>>,
        axis: u32,
        steps: i32,
    ) -> zbus::fdo::Result<()> {
        notify_input_event(
            session_handle,
            InputRequest::PointerAxisDiscrate { axis, steps },
        )
        .await
    }

    async fn notify_keyboard_keycode(
        &self,
        session_handle: ObjectPath<'_>,
        _options: HashMap<String, Value<'_>>,
        keycode: i32,
        state: u32,
    ) -> zbus::fdo::Result<()> {
        notify_input_event(
            session_handle,
            InputRequest::KeyboardKeycode { keycode, state },
        )
        .await
    }

    async fn notify_keyboard_keysym(
        &self,
        session_handle: ObjectPath<'_>,
        _options: HashMap<String, Value<'_>>,
        keysym: i32,
        state: u32,
    ) -> zbus::fdo::Result<()> {
        notify_input_event(
            session_handle,
            InputRequest::KeyboardKeysym { keysym, state },
        )
        .await
    }

    async fn notify_touch_down(
        &self,
        session_handle: ObjectPath<'_>,
        _options: HashMap<String, Value<'_>>,
        _stream: u32,
        slot: u32,
        x: f64,
        y: f64,
    ) -> zbus::fdo::Result<()> {
        notify_input_event(session_handle, InputRequest::TouchDown { slot, x, y }).await
    }

    async fn notify_touch_motion(
        &self,
        session_handle: ObjectPath<'_>,
        _options: HashMap<String, Value<'_>>,
        _stream: u32,
        slot: u32,
        x: f64,
        y: f64,
    ) -> zbus::fdo::Result<()> {
        notify_input_event(session_handle, InputRequest::TouchMotion { slot, x, y }).await
    }

    async fn notify_touch_up(
        &self,
        session_handle: ObjectPath<'_>,
        _options: HashMap<String, Value<'_>>,
        slot: u32,
    ) -> zbus::fdo::Result<()> {
        notify_input_event(session_handle, InputRequest::TouchUp { slot }).await
    }

    #[zbus(name = "ConnectToEIS")]
    async fn connect_to_eis(
        &self,
        session_handle: ObjectPath<'_>,
        _options: HashMap<String, Value<'_>>,
    ) -> zbus::fdo::Result<Fd<'_>> {
        let listener = eis::Listener::bind_auto()
            .map_err(|e| zbus::Error::Failure(format!("Failed to create EIS listener: {}", e)))?
            .ok_or_else(|| zbus::Error::Failure("Failed to create EIS listener".to_string()))?;

        let fd = io::dup(listener.as_fd()).map_err(|e| zbus::Error::Failure(e.to_string()))?;
        EIS_SERVER
            .0
            .send(EisServerMsg::NewListener(
                listener,
                session_handle.to_string(),
            ))
            .unwrap();

        Ok(Fd::from(fd))
    }
}

#[derive(Debug, Clone)]
struct RemoteInfo {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    wl_output: wl_output::WlOutput,
}

fn space_size(connection: &WayshotConnection) -> libwayshot::Size<i32> {
    let mut space_width = 0;
    let mut space_height = 0;

    let outputs = connection.get_all_outputs();
    for output in outputs {
        let libwayshot::region::Position { x, y } = output.logical_region.inner.position;
        let libwayshot::Size { width, height } = output.physical_size;
        space_width = space_width.max(width as i32 + x);
        space_height = space_height.max(height as i32 + y)
    }

    libwayshot::Size {
        width: space_width,
        height: space_height,
    }
}

fn get_monitor_info_from_socket(connection: &WayshotConnection) -> zbus::fdo::Result<RemoteInfo> {
    let libwayshot::Size { width, height } = space_size(connection);
    if SERVER_SOCK.exists() {
        let outputs = connection.get_all_outputs();
        let monitors: Vec<String> = outputs.iter().map(|output| output.name.clone()).collect();
        let index = get_selection_from_socket(monitors)?;
        let output = &outputs[index as usize];
        let libwayshot::region::Position { x, y } = output.logical_region.inner.position;
        //let libwayshot::Size { width, height } = output.physical_size;
        Ok(RemoteInfo {
            x,
            y,
            width,
            height,
            wl_output: output.wl_output.clone(),
        })
    } else {
        let info = match WaySip::new()
            .with_connection(connection.conn.clone())
            .with_selection_type(SelectionType::Screen)
            .get()
        {
            Ok(Some(info)) => info,
            Ok(None) => return Err(zbus::Error::Failure("You cancel it".to_string()).into()),
            Err(e) => return Err(zbus::Error::Failure(format!("wayland error, {e}")).into()),
        };

        let screen_info = info.screen_info;

        let libwaysip::Position { x, y } = screen_info.get_position();
        //let Size { width, height } = screen_info.get_wloutput_size();
        Ok(RemoteInfo {
            x,
            y,
            width,
            height,
            wl_output: screen_info.wl_output,
        })
    }
}
