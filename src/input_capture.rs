use std::{collections::HashMap, os::fd::AsFd};

use crate::{
    PortalResponse,
    remotedesktop::{
        EIS_SERVER, EisServerMsg, RemoteControl, RemoteInfo, RemoteSessionData, Zone,
        append_remote_session, disable_eis_listener, enable_eis_listener,
        get_monitor_info_from_socket, remote_zones,
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

#[allow(unused)]
#[derive(Debug, Type, Serialize, Deserialize)]
pub struct Position {
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
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

#[allow(unused)]
pub type BarrierId = u32;

#[derive(Type, Debug, Default, SerializeDict, DeserializeDict)]
#[zvariant(signature = "dict")]
struct GetZonesRet {
    zones: Vec<Zone>,
    zone_set: u32,
}

#[allow(unused)]
pub type FailedBarries = Vec<u32>;

pub struct InputCapture;

impl InputCapture {
    fn capabilities(&self) -> BitFlags<SupportedCapabilities> {
        SupportedCapabilities::Pointer | SupportedCapabilities::Keyboard
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

        append_remote_session(RemoteSessionData {
            session_handle: session_handle.to_string(),
            cast_thread: None,
            remote_control,
            zones: vec![Zone {
                x_offset: x,
                y_offset: y,
                width: width as u32,
                height: height as u32,
            }],
        })
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
        let zones = remote_zones(session_handle)
            .await
            .ok_or(zbus::Error::Failure("No such handle".to_owned().into()))?;
        Ok(PortalResponse::Success(GetZonesRet { zones, zone_set: 0 }))
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
        Self::activated(&cxts, session_handle, HashMap::new()).await?;
        Ok(PortalResponse::Success(EnDisableRet {}))
    }

    async fn disable(
        &self,
        session_handle: ObjectPath<'_>,
        _options: HashMap<String, Value<'_>>,
        #[zbus(signal_emitter)] cxts: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<PortalResponse<EnDisableRet>> {
        disable_eis_listener(session_handle.clone()).await;
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
        options: HashMap<String, Value<'_>>,
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
