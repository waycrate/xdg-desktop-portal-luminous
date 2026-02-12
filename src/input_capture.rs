use std::{collections::HashMap, os::fd::AsFd};

use crate::{
    PortalResponse,
    remotedesktop::{
        CursorPosition, EIS_SERVER, EisServerMsg, REMOTE_SESSIONS, RemoteControl, RemoteInfo,
        RemoteSessionData, Zone, append_remote_session, disable_eis_listener, enable_eis_listener,
        get_monitor_info_from_socket,
    },
    request::RequestInterface,
    session::{DeviceType, Session, SessionType, append_session},
};
use enumflags2::BitFlags;
use reis::eis;
use rustix::io;
use serde::{Deserialize, Serialize};
use zbus::{
    interface,
    object_server::SignalEmitter,
    zvariant::{DeserializeDict, Fd, ObjectPath, SerializeDict, Type, Value, as_value},
};

#[derive(Debug, Type, Serialize, Deserialize)]
pub struct Position {
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
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
    let remote_sessions = REMOTE_SESSIONS.lock().await;
    let session = remote_sessions
        .iter()
        .find(|session| session.session_handle == session_handle.to_string())?;
    Some((session.zone_id.value(), session.zones.clone()))
}

pub struct InputCapture;

impl InputCapture {
    fn capabilities(&self) -> BitFlags<SupportedCapabilities> {
        SupportedCapabilities::Pointer
            | SupportedCapabilities::Keyboard
            | SupportedCapabilities::TouchScreen
    }
}

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
                },
            )
            .await?;
        let current_session = Session::new(session_handle.clone(), SessionType::Remote);
        append_session(current_session.clone()).await;
        server.at(session_handle.clone(), current_session).await?;

        let remote_control = RemoteControl::init(x as u32, y as u32, width as u32, height as u32);

        append_remote_session(RemoteSessionData::new(
            session_handle.to_string(),
            None,
            remote_control,
            vec![Zone {
                x_offset: x,
                y_offset: y,
                width: width as u32,
                height: height as u32,
            }],
        ))
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
        let mut remote_sessions = REMOTE_SESSIONS.lock().await;
        let session = remote_sessions
            .iter_mut()
            .find(|session| {
                session.session_handle == session_handle.to_string()
                    // TODO: now we only accept one zone for one connection
                    && session.zone_id.value() == zone_set
            })
            .ok_or(zbus::Error::Failure("no such session".to_owned()))?;
        // TODO: here we should update the information to backend, and let the barries work
        session.barriers = valid_barries;

        Ok(PortalResponse::Success(BarrierRet { failed_barries }))
    }

    #[zbus(name = "ConnectToEIS")]
    fn connect_to_eis(
        &self,
        session_handle: ObjectPath<'_>,
        _app_id: &str,
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

    async fn enable(
        &self,
        session_handle: ObjectPath<'_>,
        _app_id: &str,
        _options: HashMap<String, Value<'_>>,
        #[zbus(signal_emitter)] cxts: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<PortalResponse<EnDisableRet>> {
        enable_eis_listener(session_handle.clone()).await;
        let mut remote_sessions = REMOTE_SESSIONS.lock().await;
        let session = remote_sessions
            .iter_mut()
            .find(|session| session.session_handle == session_handle.to_string())
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
        &self,
        session_handle: ObjectPath<'_>,
        _options: HashMap<String, Value<'_>>,
        #[zbus(signal_emitter)] cxts: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<PortalResponse<EnDisableRet>> {
        disable_eis_listener(session_handle.clone()).await;
        let remote_sessions = REMOTE_SESSIONS.lock().await;
        let session = remote_sessions
            .iter()
            .find(|session| session.session_handle == session_handle.to_string())
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
