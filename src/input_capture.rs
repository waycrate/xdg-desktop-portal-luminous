use crate::backend::get_message_sender;
use crate::dialog::Message;
use crate::eis_server::EIS_SENDER;
use crate::eis_server::EisServerMsg;
use crate::{
    PortalResponse,
    backend::{get_wlconnection, get_zbus_connection},
    remotedesktop::{
        LuminousData, RESTORE_DATA_VERSION, RemoteInfo, RestoreData, VENDOR_NAME,
        get_monitor_info_from_socket, space_size,
    },
    request::RequestInterface,
    session::{DeviceType, Session, SessionType, append_session},
};
use crate::{
    session::{PersistMode, SESSIONS},
    utils::{InputEvent, InputRequest},
};
use enumflags2::BitFlags;
#[allow(unused)]
use futures::{SinkExt, channel::mpsc::Sender as FutSender};
use reis::eis;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{self, AtomicU32};
use std::sync::{Arc, LazyLock};
use std::{
    collections::HashMap,
    os::{fd::AsFd, unix::net::UnixStream},
};
use tokio::sync::Mutex;
use zbus::{
    interface,
    object_server::SignalEmitter,
    zvariant::{
        DeserializeDict, Fd, ObjectPath, SerializeDict, Type, Value,
        as_value::{self, optional},
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
/// The id of the window.
///
/// Internally Iced reserves `window::Id::MAIN` for the first window spawned.
pub struct ZoneId(u32);

static COUNT: AtomicU32 = AtomicU32::new(0);

impl ZoneId {
    /// Creates a new unique window [`Id`].
    pub fn unique() -> ZoneId {
        ZoneId(COUNT.fetch_add(1, atomic::Ordering::Relaxed))
    }
    pub fn value(&self) -> u32 {
        self.0
    }
}
pub async fn enable_ei_client(session_handle: ObjectPath<'_>) {
    EIS_SENDER
        .send(EisServerMsg::ActiveContext(session_handle.to_string()))
        .unwrap();
}
pub async fn disable_ei_client(session_handle: ObjectPath<'_>) {
    EIS_SENDER
        .send(EisServerMsg::StopContext(session_handle.to_string()))
        .unwrap();
}
#[derive(Debug, Type, Serialize, Deserialize)]
pub struct Position {
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
}

#[derive(Debug, Type, Serialize, Deserialize, Clone, Copy)]
pub struct Zone {
    pub width: u32,
    pub height: u32,
    pub x_offset: i32,
    pub y_offset: i32,
}
#[derive(Default, Debug, Clone, Copy, Serialize, Deserialize, Type)]
pub struct CursorPosition {
    x: f64,
    y: f64,
}
pub struct InputCaptureData {
    pub session_handle: String,
    pub zones: Vec<Zone>,
    pub zone_id: ZoneId,
    pub barriers: Vec<BarrierInfo>,
    cursor: CursorPosition,
    activation_id: u32,
}

impl InputCaptureData {
    fn stop(&self) {
        let _ = EIS_SENDER.send(EisServerMsg::RemoveContext(self.session_handle.clone()));
    }
    pub fn step(&mut self) {
        self.activation_id += 1;
    }
    pub fn activation_id(&self) -> u32 {
        self.activation_id
    }
    pub fn cursor_position(&self) -> CursorPosition {
        self.cursor
    }

    pub fn update_cursor(&mut self, event: InputRequest) {
        match event {
            InputRequest::PointerMotionAbsolute { x, y } => {
                self.cursor = CursorPosition { x, y };
            }
            InputRequest::PointerMotion { dx, dy } => {
                self.cursor.x += dx;
                self.cursor.y += dy;
            }
            _ => {}
        }
    }
}

pub async fn handle_input_event(
    InputEvent {
        session_handle,
        request,
    }: InputEvent,
) {
    if let Ok(path) = ObjectPath::try_from(session_handle) {
        let _ = update_session_data(path, request).await;
    }
}

// TODO: need to check the position and send the activation event
async fn update_session_data(session_handle: ObjectPath<'_>, event: InputRequest) -> Option<()> {
    let mut capture_sessions = INPUT_CAPTURE_SESSIONS.lock().await;
    let session = capture_sessions.get_mut(session_handle.as_str())?;
    session.update_cursor(event);
    let cursor_position = session.cursor_position();
    let activation_id = session.activation_id();
    let connection = get_zbus_connection();
    let signal_context =
        SignalEmitter::new(&connection, "/org/freedesktop/portal/desktop").unwrap();
    for barrier in &mut session.barriers {
        if barrier.through(cursor_position) && barrier.status != BarrierStatus::Activated {
            let _ = InputCapture::activated(
                &signal_context,
                session_handle.as_ref(),
                ActivatedSignal {
                    activation_id,
                    cursor_position,
                    barrier_id: barrier.barrier_id,
                },
            )
            .await;
            barrier.status = BarrierStatus::Activated;
        }
    }
    Some(())
}

pub static INPUT_CAPTURE_SESSIONS: LazyLock<Arc<Mutex<HashMap<String, InputCaptureData>>>> =
    LazyLock::new(|| Arc::new(Mutex::new(HashMap::new())));

pub async fn append_capture_session(path: &str, session: InputCaptureData) {
    let mut sessions = INPUT_CAPTURE_SESSIONS.lock().await;
    sessions.insert(path.to_string(), session);
}

pub async fn remove_capture_session(session_handle: ObjectPath<'_>) {
    let mut sender = get_message_sender();
    let _ = sender
        .send(Message::StopCapture(session_handle.to_string()))
        .await;
    let mut sessions = INPUT_CAPTURE_SESSIONS.lock().await;
    let Some(session) = sessions.remove(session_handle.as_str()) else {
        return;
    };
    session.stop();
    tracing::info!("session {} is stopped", session.session_handle);
}

impl Position {
    fn legal_check(&self) -> bool {
        if self.x1 == self.x2 {
            return true;
        }
        if self.y1 == self.y2 {
            return true;
        }
        false
    }
}

pub type SupportedCapabilities = DeviceType;

#[derive(Type, Debug, Default, Serialize, Deserialize)]
#[zvariant(signature = "dict")]
struct CreateSessionOptions {
    #[serde(with = "as_value")]
    capabilities: BitFlags<SupportedCapabilities>,
}

#[derive(Type, Debug, Default, Serialize, Deserialize)]
#[zvariant(signature = "dict")]
struct StartSessionOptions {
    #[serde(with = "as_value")]
    capabilities: BitFlags<SupportedCapabilities>,
    #[serde(with = "as_value", default)]
    persist_mode: PersistMode,
    #[serde(with = "optional", skip_serializing_if = "Option::is_none", default)]
    restore_data: Option<RestoreData>,
}

#[derive(Type, Debug, Default, Serialize, Deserialize)]
#[zvariant(signature = "dict")]
struct StartResult {
    #[serde(with = "as_value")]
    capabilities: BitFlags<SupportedCapabilities>,
    #[serde(with = "as_value")]
    clipboard_enabled: bool,
    #[serde(with = "optional", skip_serializing_if = "Option::is_none", default)]
    restore_data: Option<RestoreData>,
}

#[derive(Type, Debug, Default, Serialize, Deserialize)]
#[zvariant(signature = "dict")]
struct CreateSessionRet {
    #[serde(with = "as_value")]
    capabilities: BitFlags<SupportedCapabilities>,
    #[serde(with = "as_value")]
    session_id: String,
}

#[derive(Type, Debug, Default, Serialize, Deserialize)]
#[zvariant(signature = "dict")]
struct CreateSessionRet2 {}
#[derive(Type, Debug, Default, Serialize, Deserialize)]
#[zvariant(signature = "dict")]
struct EnDisableRet {}

#[derive(Type, Debug, Default, Serialize, Deserialize)]
#[zvariant(signature = "dict")]
struct ActivatedSignal {
    #[serde(with = "as_value")]
    activation_id: u32,
    #[serde(with = "as_value")]
    cursor_position: CursorPosition,
    #[serde(with = "as_value")]
    barrier_id: BarrierId,
}

#[derive(Type, Debug, Default, Serialize, Deserialize)]
#[zvariant(signature = "dict")]
struct DeactivatedSignal {
    #[serde(with = "as_value")]
    activation_id: u32,
    #[serde(with = "as_value")]
    cursor_position: CursorPosition,
}

#[derive(Type, Debug, Default, SerializeDict, DeserializeDict)]
#[zvariant(signature = "dict")]
struct GetZonesRet {
    zones: Vec<Zone>,
    zone_set: u32,
}

pub type BarrierId = u32;

#[derive(Debug, Type, Serialize, Deserialize, Default, PartialEq, Eq, Copy, Clone)]
pub enum BarrierStatus {
    Activated,
    Deactivated,
    #[default]
    Null,
}

#[derive(Debug, Type, Serialize, Deserialize, Default, PartialEq, Eq, Copy, Clone)]
pub enum CapturePlace {
    #[default]
    Right,
    Left,
    Top,
    Bottom,
}

// I need another enum to mark the position of the display
// But why not active them all?
#[derive(Debug, Type, Serialize, Deserialize)]
#[zvariant(signature = "dict")]
pub struct BarrierInfo {
    #[serde(with = "as_value")]
    barrier_id: BarrierId,
    #[serde(with = "as_value")]
    position: Position,
    // just used to record something
    // default is false
    #[serde(with = "as_value", default)]
    status: BarrierStatus,
    #[serde(with = "as_value", default)]
    placement: CapturePlace,
}

impl BarrierInfo {
    fn valid(&self) -> bool {
        self.position.legal_check()
    }

    fn check_placement(&mut self, zone: Zone) {
        // vertical
        if self.position.x1 == self.position.x2 {
            if self.position.x1 <= zone.x_offset {
                self.placement = CapturePlace::Left;
            } else {
                self.placement = CapturePlace::Right;
            }
            return;
        }
        // horizontal
        if self.position.y1 <= zone.y_offset {
            self.placement = CapturePlace::Top;
        } else {
            self.placement = CapturePlace::Bottom;
        }
    }

    fn check_output(&self, output_info: &libwayshot::OutputInfo) -> bool {
        match self.placement {
            CapturePlace::Right | CapturePlace::Bottom => {
                output_info.logical_position().x == self.position.x1
                    && output_info.logical_position().y == self.position.y1
            }
            placement => {
                let logical_position = output_info.logical_position();
                let logical_size = output_info.logical_size();
                if placement == CapturePlace::Left {
                    logical_position.x + logical_size.width as i32 == self.position.x1
                        && logical_position.y == self.position.y1
                } else {
                    logical_position.x == self.position.x1
                        && logical_position.y + logical_size.height as i32 == self.position.y1
                }
            }
        }
    }

    fn through(&self, position: CursorPosition) -> bool {
        let position_x = position.x as i32;
        let position_y = position.y as i32;
        match self.placement {
            CapturePlace::Right => position_x > self.position.x1,
            CapturePlace::Left => position_x < self.position.x1,
            CapturePlace::Top => position_y < self.position.y1,
            CapturePlace::Bottom => position_y > self.position.y1,
        }
    }
}

#[derive(Debug, Type, Serialize, Deserialize)]
#[zvariant(signature = "dict")]
struct BarrierRet {
    #[serde(with = "as_value")]
    failed_barries: Vec<u32>,
}

async fn capture_zones(session_handle: ObjectPath<'_>) -> Option<(u32, Vec<Zone>)> {
    let remote_sessions = INPUT_CAPTURE_SESSIONS.lock().await;
    let session = remote_sessions.get(session_handle.as_str())?;
    Some((session.zone_id.value(), session.zones.clone()))
}

pub struct InputCapture {
    pub sender: FutSender<Message>,
    clients: HashMap<String, UnixStream>,
}

impl InputCapture {
    pub fn new(sender: FutSender<Message>) -> Self {
        Self {
            sender,
            clients: HashMap::new(),
        }
    }
    fn capabilities(&self) -> BitFlags<SupportedCapabilities> {
        SupportedCapabilities::Pointer
            | SupportedCapabilities::Keyboard
            | SupportedCapabilities::TouchScreen
    }
}

// NOTE: because it is broken, so about the whitelist, will do it later
#[interface(name = "org.freedesktop.impl.portal.InputCapture")]
impl InputCapture {
    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        2
    }
    #[zbus(property)]
    fn supported_capabilities(&self) -> u32 {
        self.capabilities().bits()
    }

    // NOTE: this interface won't be used anymore
    async fn create_session(
        &mut self,
        handle: ObjectPath<'_>,
        session_handle: ObjectPath<'_>,
        app_id: &str,
        _parent_window: &str,
        options: CreateSessionOptions,
        #[zbus(object_server)] server: &zbus::ObjectServer,
    ) -> zbus::fdo::Result<PortalResponse<CreateSessionRet>> {
        if (options.capabilities | self.capabilities()) != self.capabilities() {
            return Err(zbus::Error::Failure("Unsupported capability".to_owned()).into());
        }
        let connection =
            libwayshot::WayshotConnection::from_connection(get_wlconnection()).unwrap();
        let RemoteInfo {
            width,
            height,
            x,
            y,
            ..
        } = get_monitor_info_from_socket(&connection)?;
        let capabilities = options.capabilities & self.capabilities();
        tracing::info!(
            "Start session: path :{}, appid: {}",
            handle.as_str(),
            app_id
        );
        server
            .at(
                handle.clone(),
                RequestInterface {
                    handle_path: handle.clone().into(),
                    close_action: None,
                },
            )
            .await?;
        let current_session = Session::new(session_handle.clone(), SessionType::InputCapture);
        append_session(current_session.clone()).await;
        server.at(session_handle.clone(), current_session).await?;

        append_capture_session(
            &session_handle,
            InputCaptureData {
                session_handle: session_handle.to_string(),
                zones: vec![Zone {
                    x_offset: x,
                    y_offset: y,
                    width,
                    height,
                }],
                zone_id: ZoneId::unique(),
                barriers: vec![],
                activation_id: 0,
                cursor: CursorPosition::default(),
            },
        )
        .await;
        Ok(PortalResponse::Success(CreateSessionRet {
            capabilities,
            session_id: session_handle.to_string(),
        }))
    }

    // here open a layershell to capture all the events
    async fn create_session2(
        &self,
        session_handle: ObjectPath<'_>,
        _app_id: &str,
        _options: HashMap<String, Value<'_>>,
        #[zbus(object_server)] server: &zbus::ObjectServer,
    ) -> zbus::fdo::Result<CreateSessionRet2> {
        let current_session = Session::new(session_handle.clone(), SessionType::InputCapture);
        // TODO: check the app_id
        append_session(current_session.clone()).await;
        server.at(session_handle.clone(), current_session).await?;

        Ok(CreateSessionRet2 {})
    }

    async fn start(
        &mut self,
        _handle: ObjectPath<'_>,
        session_handle: ObjectPath<'_>,
        _app_id: &str,
        _parent_window: &str,
        options: StartSessionOptions,
        #[zbus(connection)] dbus_connection: &zbus::Connection,
    ) -> zbus::fdo::Result<PortalResponse<StartResult>> {
        let locked_sessions = SESSIONS.lock().await;
        let Some(index) = locked_sessions
            .iter()
            .position(|this_session| this_session.handle_path == session_handle.clone().into())
        else {
            tracing::warn!("No session is created or it is removed");
            return Ok(PortalResponse::Other);
        };

        let current_session = &locked_sessions[index];

        if (options.capabilities | self.capabilities()) != self.capabilities() {
            return Err(zbus::Error::Failure("Unsupported capability".to_owned()).into());
        }
        let connection =
            libwayshot::WayshotConnection::from_connection(get_wlconnection()).unwrap();
        let RemoteInfo {
            width,
            height,
            x,
            y,
            output_name,
            ..
        } = if let Some(RestoreData {
            vendor_name,
            version,
            data,
        }) = options.restore_data
            && options.persist_mode.is_persist()
            && vendor_name == VENDOR_NAME
            && version == RESTORE_DATA_VERSION
            && let Some(display) = connection
                .get_all_outputs()
                .iter()
                .find(|output_info| output_info.name == data.display)
        {
            let libwayshot::Size {
                width: space_width,
                height: space_height,
            } = space_size(&connection);

            let libwayshot::region::Position { x, y } = display.logical_region.inner.position;
            let libwayshot::region::Size { width, height } = display.logical_region.inner.size;
            RemoteInfo {
                x,
                y,
                width,
                height,
                space_width,
                space_height,
                output_name: display.name.to_owned(),
                wl_output: display.wl_output.clone(),
            }
        } else {
            get_monitor_info_from_socket(&connection)?
        };
        let capabilities = options.capabilities & self.capabilities();
        let restore_data = options.persist_mode.is_persist().then(|| {
            RestoreData::new(LuminousData {
                display: output_name,
            })
        });
        let _ = current_session;
        drop(locked_sessions);
        append_capture_session(
            &session_handle,
            InputCaptureData {
                session_handle: session_handle.to_string(),
                zones: vec![Zone {
                    x_offset: x,
                    y_offset: y,
                    width,
                    height,
                }],
                zone_id: ZoneId::unique(),
                barriers: vec![],
                activation_id: 0,
                cursor: CursorPosition::default(),
            },
        )
        .await;
        let clipboard_enabled =
            crate::clipboard::ensure_clipboard_session(&session_handle, dbus_connection.clone())
                .await;
        Ok(PortalResponse::Success(StartResult {
            capabilities,
            clipboard_enabled,
            restore_data,
        }))
    }

    async fn get_zones(
        &self,
        _handle: ObjectPath<'_>,
        session_handle: ObjectPath<'_>,
        _app_id: &str,
        _options: HashMap<String, Value<'_>>,
    ) -> zbus::fdo::Result<PortalResponse<GetZonesRet>> {
        let (zone_set, zones) = capture_zones(session_handle)
            .await
            .ok_or(zbus::Error::Failure("No such handle".to_owned()))?;
        Ok(PortalResponse::Success(GetZonesRet { zones, zone_set }))
    }

    // NOTE: this place deskflow will set the barriers here. We should only accept the one that the
    // virtual desktop has
    async fn set_pointer_barriers(
        &mut self,
        _handle: ObjectPath<'_>,
        session_handle: ObjectPath<'_>,
        _app_id: &str,
        _options: HashMap<String, Value<'_>>,
        barriers: Vec<BarrierInfo>,
        zone_set: u32,
    ) -> zbus::fdo::Result<PortalResponse<BarrierRet>> {
        let connection =
            libwayshot::WayshotConnection::from_connection(get_wlconnection()).unwrap();
        let mut valid_outputs = vec![];
        let mut failed_barries = vec![];
        let mut valid_barries = vec![];
        let available_outputs = connection.get_all_outputs();
        let mut capture_sessions = INPUT_CAPTURE_SESSIONS.lock().await;
        let session = capture_sessions
            .get_mut(session_handle.as_str())
            .ok_or(zbus::Error::Failure("no such session".to_owned()))?;

        if session.zone_id.value() != zone_set {
            return Err(zbus::fdo::Error::ZBus(zbus::Error::Failure(
                "no such session".to_owned(),
            )));
        }
        // NOTE: because we only have one zone, so it is safe
        let zone = session.zones[0];
        for mut barrier in barriers {
            barrier.check_placement(zone);
            // NOTE: only accept if there is a virtual display can accept the region
            if barrier.valid()
                && let Some(output) = available_outputs
                    .iter()
                    .find(|output| barrier.check_output(output))
            {
                valid_barries.push(barrier);
                valid_outputs.push(output);
            } else {
                failed_barries.push(barrier.barrier_id);
            }
        }

        session.barriers = valid_barries;
        for output in valid_outputs {
            let _ = self
                .sender
                .send(Message::CaptureLayer {
                    wl_output: output.wl_output.clone(),
                    handle: session_handle.to_string(),
                    position: output.logical_position(),
                    size: output.logical_size(),
                })
                .await;
        }

        Ok(PortalResponse::Success(BarrierRet { failed_barries }))
    }

    #[zbus(name = "ConnectToEIS")]
    fn connect_to_eis(
        &mut self,
        session_handle: ObjectPath<'_>,
        _app_id: &str,
        _options: HashMap<String, Value<'_>>,
    ) -> zbus::fdo::Result<Fd<'_>> {
        let listener = eis::Listener::bind_auto()
            .map_err(|e| zbus::Error::Failure(format!("Failed to create EIS listener: {}", e)))?;
        let path = listener.path();
        use std::os::unix::net::UnixStream;
        let stream = UnixStream::connect(path).map_err(|e| {
            zbus::Error::Failure(format!("Failed to open unix stream: {path:?} with {e}"))
        })?;

        self.clients.insert(session_handle.to_string(), stream);

        EIS_SENDER
            .send(EisServerMsg::NewListener(
                listener,
                session_handle.to_string(),
            ))
            .unwrap();

        Ok(Fd::from(self.clients[session_handle.as_str()].as_fd()))
    }

    async fn enable(
        &self,
        session_handle: ObjectPath<'_>,
        _app_id: &str,
        _options: HashMap<String, Value<'_>>,
        //#[zbus(signal_emitter)] cxts: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<PortalResponse<EnDisableRet>> {
        enable_ei_client(session_handle.clone()).await;
        let mut capture_sessions = INPUT_CAPTURE_SESSIONS.lock().await;
        let session = capture_sessions
            .get_mut(session_handle.as_str())
            .ok_or(zbus::Error::Failure("no such session".to_owned()))?;
        session.step();
        Ok(PortalResponse::Success(EnDisableRet {}))
    }

    async fn disable(
        &mut self,
        session_handle: ObjectPath<'_>,
        _options: HashMap<String, Value<'_>>,
        #[zbus(signal_emitter)] cxts: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<PortalResponse<EnDisableRet>> {
        self.clients.remove(session_handle.as_str());
        disable_ei_client(session_handle.clone()).await;
        let capture_sessions = INPUT_CAPTURE_SESSIONS.lock().await;
        let session = capture_sessions
            .get(session_handle.as_str())
            .ok_or(zbus::Error::Failure("no such session".to_owned()))?;

        let _ = Self::deactivated(
            &cxts,
            session_handle.as_ref(),
            DeactivatedSignal {
                activation_id: session.activation_id(),
                cursor_position: session.cursor_position(),
            },
        )
        .await;
        Self::disabled(&cxts, session_handle, HashMap::new()).await?;
        Ok(PortalResponse::Success(EnDisableRet {}))
    }

    #[zbus(signal)]
    async fn disabled(
        signal_ctx: &SignalEmitter<'_>,
        session_handle: ObjectPath<'_>,
        options: HashMap<String, Value<'_>>,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn activated(
        signal_ctx: &SignalEmitter<'_>,
        session_handle: ObjectPath<'_>,
        options: ActivatedSignal,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn deactivated(
        signal_ctx: &SignalEmitter<'_>,
        session_handle: ObjectPath<'_>,
        options: DeactivatedSignal,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn zone_changed(
        signal_ctx: &SignalEmitter<'_>,
        session_handle: ObjectPath<'_>,
        options: Vec<Zone>,
    ) -> zbus::Result<()>;
}
