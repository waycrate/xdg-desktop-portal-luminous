use std::{
    collections::HashMap,
    os::{fd::AsFd, unix::net::UnixStream},
};
mod ei_client;
use crate::utils::InputRequest;
use crate::{
    PortalResponse,
    remotedesktop::{RemoteInfo, get_monitor_info_from_socket},
    request::RequestInterface,
    session::{DeviceType, Session, SessionType, append_session},
};
use calloop::channel::Sender;
use ei_client::EiClientMsg;
use enumflags2::BitFlags;
use reis::{ei, eis};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{self, AtomicU32};
use std::sync::{Arc, LazyLock};
use tokio::sync::Mutex;
use zbus::{
    interface,
    object_server::SignalEmitter,
    zvariant::{DeserializeDict, Fd, ObjectPath, SerializeDict, Type, Value, as_value},
};
type EiClientSender = Sender<EiClientMsg>;
pub static EI_CLIENT: LazyLock<EiClientSender> = LazyLock::new(ei_client::start);

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
    EI_CLIENT
        .send(EiClientMsg::ActiveContext(session_handle.to_string()))
        .unwrap();
}
pub async fn disable_ei_client(session_handle: ObjectPath<'_>) {
    EI_CLIENT
        .send(EiClientMsg::StopContext(session_handle.to_string()))
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
    pub fn step(&mut self) {
        self.activation_id += 1;
    }
    pub fn activation_id(&self) -> u32 {
        self.activation_id
    }
    pub fn cursor_position(&self) -> CursorPosition {
        self.cursor
    }
    #[allow(unused)]
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

pub static INPUT_CAPTURE_SESSIONS: LazyLock<Arc<Mutex<HashMap<String, InputCaptureData>>>> =
    LazyLock::new(|| Arc::new(Mutex::new(HashMap::new())));

pub async fn append_capture_session(path: &str, session: InputCaptureData) {
    let mut sessions = INPUT_CAPTURE_SESSIONS.lock().await;
    sessions.insert(path.to_string(), session);
}

pub async fn remove_capture_session(session_handle: ObjectPath<'_>) {
    let mut sessions = INPUT_CAPTURE_SESSIONS.lock().await;
    let Some(session) = sessions.remove(session_handle.as_str()) else {
        return;
    };
    tracing::info!("session {} is stopped", session.session_handle);
    disable_ei_client(session_handle).await;
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
struct CreateSessionRet {
    #[serde(with = "as_value")]
    capabilities: BitFlags<SupportedCapabilities>,
    #[serde(with = "as_value")]
    session_id: String,
}
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
struct DisableSignal {
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
#[derive(Debug, Type, Serialize, Deserialize)]
#[zvariant(signature = "dict")]
pub struct BarrierInfo {
    #[serde(with = "as_value")]
    barrier_id: BarrierId,
    #[serde(with = "as_value")]
    position: Position,
}

impl BarrierInfo {
    fn valid(&self) -> bool {
        self.position.legal_check()
    }
}

#[derive(Debug, Type, Serialize, Deserialize)]
#[zvariant(signature = "dict")]
struct BarrierRet {
    #[serde(with = "as_value")]
    failed_barries: Vec<u32>,
}

async fn remote_zones(session_handle: ObjectPath<'_>) -> Option<(u32, Vec<Zone>)> {
    let remote_sessions = INPUT_CAPTURE_SESSIONS.lock().await;
    let session = remote_sessions.get(session_handle.as_str())?;
    Some((session.zone_id.value(), session.zones.clone()))
}

#[derive(Default)]
pub struct InputCapture {
    clients: HashMap<String, UnixStream>,
}

impl InputCapture {
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
        1
    }
    #[zbus(property)]
    fn supported_capabilities(&self) -> u32 {
        self.capabilities().bits()
    }
    async fn create_session(
        &self,
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
        let connection = libwayshot::WayshotConnection::new().unwrap();
        let RemoteInfo {
            width,
            height,
            x,
            y,
            ..
        } = get_monitor_info_from_socket(&connection)?;
        let capabilities = options.capabilities & self.capabilities();
        tracing::info!("Start shot: path :{}, appid: {}", handle.as_str(), app_id);
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
                    width: width as u32,
                    height: height as u32,
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

    async fn get_zones(
        &self,
        _handle: ObjectPath<'_>,
        session_handle: ObjectPath<'_>,
        _app_id: &str,
        _options: HashMap<String, Value<'_>>,
    ) -> zbus::fdo::Result<PortalResponse<GetZonesRet>> {
        let (zone_set, zones) = remote_zones(session_handle)
            .await
            .ok_or(zbus::Error::Failure("No such handle".to_owned()))?;
        Ok(PortalResponse::Success(GetZonesRet { zones, zone_set }))
    }

    async fn set_pointer_barriers(
        &self,
        _handle: ObjectPath<'_>,
        session_handle: ObjectPath<'_>,
        _app_id: &str,
        _options: HashMap<String, Value<'_>>,
        barriers: Vec<BarrierInfo>,
        zone_set: u32,
    ) -> zbus::fdo::Result<PortalResponse<BarrierRet>> {
        let mut valid_barries = vec![];
        let mut failed_barries = vec![];
        for barrier in barriers {
            if barrier.valid() {
                valid_barries.push(barrier);
            } else {
                failed_barries.push(barrier.barrier_id);
            }
        }
        let mut capture_sessions = INPUT_CAPTURE_SESSIONS.lock().await;
        let session = capture_sessions
            .get_mut(session_handle.as_str())
            .ok_or(zbus::Error::Failure("no such session".to_owned()))?;
        if session.zone_id.value() != zone_set {
            return Err(zbus::fdo::Error::ZBus(zbus::Error::Failure(
                "no such session".to_owned(),
            )));
        }
        // TODO: here we should update the information to backend, and let the barries work
        session.barriers = valid_barries;

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
        let stream_server = UnixStream::connect(path).map_err(|e| {
            zbus::Error::Failure(format!("Failed to open unix stream: {path:?} with {e}"))
        })?;
        let stream_client = UnixStream::connect(path).map_err(|e| {
            zbus::Error::Failure(format!("Failed to open unix stream: {path:?} with {e}"))
        })?;
        let context = ei::Context::new(stream_client).unwrap();
        self.clients
            .insert(session_handle.to_string(), stream_server);

        EI_CLIENT
            .send(EiClientMsg::NewContext(context, session_handle.to_string()))
            .unwrap();

        Ok(Fd::from(self.clients[session_handle.as_str()].as_fd()))
    }

    async fn enable(
        &self,
        session_handle: ObjectPath<'_>,
        _app_id: &str,
        _options: HashMap<String, Value<'_>>,
        #[zbus(signal_emitter)] cxts: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<PortalResponse<EnDisableRet>> {
        enable_ei_client(session_handle.clone()).await;
        let mut remote_sessions = INPUT_CAPTURE_SESSIONS.lock().await;
        let session = remote_sessions
            .get_mut(session_handle.as_str())
            .ok_or(zbus::Error::Failure("no such session".to_owned()))?;
        session.step();
        Self::activated(
            &cxts,
            session_handle,
            ActivatedSignal {
                activation_id: session.activation_id(),
                cursor_position: session.cursor_position(),
                // TODO: I should check it
                barrier_id: 0,
            },
        )
        .await?;
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
        let remote_sessions = INPUT_CAPTURE_SESSIONS.lock().await;
        let session = remote_sessions
            .get(session_handle.as_str())
            .ok_or(zbus::Error::Failure("no such session".to_owned()))?;
        Self::disabled(
            &cxts,
            session_handle,
            DisableSignal {
                activation_id: session.activation_id(),
                cursor_position: session.cursor_position(),
            },
        )
        .await?;
        Ok(PortalResponse::Success(EnDisableRet {}))
    }

    #[zbus(signal)]
    async fn disabled(
        signal_ctx: &SignalEmitter<'_>,
        session_handle: ObjectPath<'_>,
        options: DisableSignal,
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
        options: HashMap<String, Value<'_>>,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn zone_changed(
        signal_ctx: &SignalEmitter<'_>,
        session_handle: ObjectPath<'_>,
        options: Vec<Zone>,
    ) -> zbus::Result<()>;
}
