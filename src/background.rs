use std::{
    collections::{HashMap, hash_map::Entry},
    io,
    path::PathBuf,
    sync::{Arc, Mutex as StdMutex},
};

use futures::{
    SinkExt, StreamExt,
    channel::mpsc::{Sender, UnboundedReceiver},
};
use libwayshot::WayshotConnection;
use serde::Serialize;
use tokio::sync::{Mutex, oneshot};
use zbus::{
    fdo, interface,
    object_server::SignalEmitter,
    zvariant::{ObjectPath, OwnedObjectPath, Type, as_value},
};

use crate::{
    PortalResponse,
    dialog::{CopySelect, Message},
    request::{RequestCloseAction, RequestInterface},
    settings::SETTING_CONFIG,
    systemd::Systemd1Proxy,
    utils::XDG_CONFIG_HOME,
};

const AUTOSTART_DBUS_ACTIVATABLE: u32 = 0x1;

#[derive(Debug)]
pub struct BackgroundBackend {
    pub sender: Sender<Message>,
    pub pending_responses: PendingBackgroundResponses,
}

pub type PendingBackgroundResponses = Arc<Mutex<PendingBackgroundResponseRegistry>>;

#[derive(Debug, Default)]
pub struct PendingBackgroundResponseRegistry {
    by_handle: HashMap<String, PendingBackgroundRequest>,
    app_handles: HashMap<String, Vec<String>>,
}

#[derive(Debug)]
struct PendingBackgroundRequest {
    app_id: String,
    response_sender: oneshot::Sender<u32>,
}

#[derive(Debug, PartialEq, Eq)]
enum PendingBackgroundReservation {
    Prompt,
    Coalesced,
    DuplicateHandle,
}

impl PendingBackgroundResponseRegistry {
    fn reserve(
        &mut self,
        handle: String,
        app_id: &str,
        response_sender: oneshot::Sender<u32>,
    ) -> PendingBackgroundReservation {
        if self.by_handle.contains_key(&handle) {
            return PendingBackgroundReservation::DuplicateHandle;
        }

        let prompt_needed = !self.app_handles.contains_key(app_id);
        self.by_handle.insert(
            handle.clone(),
            PendingBackgroundRequest {
                app_id: app_id.to_owned(),
                response_sender,
            },
        );
        self.app_handles
            .entry(app_id.to_owned())
            .or_default()
            .push(handle);

        if prompt_needed {
            PendingBackgroundReservation::Prompt
        } else {
            PendingBackgroundReservation::Coalesced
        }
    }

    fn take_app_responses_by_handle(&mut self, handle: &str) -> Vec<oneshot::Sender<u32>> {
        let Some(app_id) = self
            .by_handle
            .get(handle)
            .map(|request| request.app_id.clone())
        else {
            return Vec::new();
        };

        self.remove_app(&app_id)
    }

    fn remove_request(&mut self, handle: &str, app_id: &str) {
        let Some(handles) = self.app_handles.get_mut(app_id) else {
            self.by_handle.remove(handle);
            return;
        };

        let primary_handle = handles.first().is_some_and(|primary| primary == handle);
        if primary_handle {
            self.remove_app(app_id);
            return;
        }

        handles.retain(|pending_handle| pending_handle != handle);
        if handles.is_empty() {
            self.app_handles.remove(app_id);
        }
        self.by_handle.remove(handle);
    }

    fn remove_app(&mut self, app_id: &str) -> Vec<oneshot::Sender<u32>> {
        let Some(handles) = self.app_handles.remove(app_id) else {
            return Vec::new();
        };

        handles
            .into_iter()
            .filter_map(|handle| {
                self.by_handle
                    .remove(&handle)
                    .map(|request| request.response_sender)
            })
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Type)]
#[zvariant(signature = "v")]
#[repr(u32)]
pub enum AppStatus {
    Background = 0,
    Running,
    // Kept for the Background portal status enum, but currently not reported:
    // ext-foreign-toplevel-list does not expose activation/focus state.
    #[allow(dead_code)]
    Active,
}

impl Serialize for AppStatus {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        zbus::zvariant::Value::U32(*self as u32).serialize(serializer)
    }
}

#[derive(Clone, Copy, Debug, Type, Serialize)]
#[zvariant(signature = "dict")]
pub struct NotifyBackgroundResult {
    #[serde(with = "as_value")]
    result: u32,
}

#[interface(name = "org.freedesktop.impl.portal.Background")]
impl BackgroundBackend {
    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        2
    }

    async fn notify_background(
        &self,
        handle: ObjectPath<'_>,
        app_id: String,
        name: String,
        #[zbus(object_server)] server: &zbus::ObjectServer,
    ) -> fdo::Result<PortalResponse<NotifyBackgroundResult>> {
        tracing::info!(
            "Background request: path: {}, appid: {}, name: {}",
            handle.as_str(),
            app_id,
            name
        );

        match SETTING_CONFIG
            .lock()
            .await
            .background_permission_default
            .as_str()
        {
            "allow" => {
                return Ok(PortalResponse::Success(NotifyBackgroundResult {
                    result: 1,
                }));
            }
            "deny" => {
                return Ok(PortalResponse::Success(NotifyBackgroundResult {
                    result: 0,
                }));
            }
            _ => {}
        }

        let handle_path: OwnedObjectPath = handle.clone().into();
        let handle_name = handle.as_str().to_owned();
        let app_id_for_cleanup = app_id.clone();
        let (cancel_sender, mut cancel_receiver) = oneshot::channel();
        let (response_sender, response_receiver) = oneshot::channel();

        let should_prompt = {
            let mut pending_responses = self.pending_responses.lock().await;
            match pending_responses.reserve(handle_name.clone(), &app_id, response_sender) {
                PendingBackgroundReservation::Prompt => true,
                PendingBackgroundReservation::Coalesced => false,
                PendingBackgroundReservation::DuplicateHandle => {
                    return Err(fdo::Error::InvalidArgs(format!(
                        "duplicate background request handle: {handle_name}"
                    )));
                }
            }
        };
        let close_action = RequestCloseAction {
            cancel_sender: Arc::new(StdMutex::new(Some(cancel_sender))),
            ui_sender: self.sender.clone(),
            close_message: should_prompt.then(|| Message::CloseBackgroundPrompt {
                handle: handle_name.clone(),
            }),
        };

        if let Err(e) = server
            .at(
                handle.clone(),
                RequestInterface {
                    handle_path: handle_path.clone(),
                    close_action: Some(close_action),
                },
            )
            .await
        {
            self.pending_responses
                .lock()
                .await
                .remove_request(&handle_name, &app_id_for_cleanup);
            return Err(e.into());
        }

        let prompt_result = if should_prompt {
            self.sender
                .clone()
                .send(Message::BackgroundPrompt {
                    handle: handle_name.clone(),
                    app_id,
                    name,
                })
                .await
                .map_err(|e| e.to_string())
        } else {
            Ok(())
        };
        if let Err(e) = prompt_result {
            self.pending_responses
                .lock()
                .await
                .remove_request(&handle_name, &app_id_for_cleanup);
            let _ = server
                .remove::<RequestInterface, &OwnedObjectPath>(&handle_path)
                .await;
            return Err(zbus::Error::Failure(e).into());
        }

        let response = tokio::select! {
            biased;
            response = response_receiver => {
                match response {
                    Ok(result) => {
                        Ok(PortalResponse::Success(NotifyBackgroundResult { result }))
                    }
                    Err(_) => {
                        Ok(PortalResponse::Cancelled)
                    }
                }
            }
            _ = &mut cancel_receiver => {
                Ok(PortalResponse::Cancelled)
            }
        };

        self.pending_responses
            .lock()
            .await
            .remove_request(&handle_name, &app_id_for_cleanup);
        let _ = server
            .remove::<RequestInterface, &OwnedObjectPath>(&handle_path)
            .await;

        response
    }

    async fn enable_autostart(
        &self,
        app_id: String,
        enable: bool,
        commandline: Vec<String>,
        flags: u32,
    ) -> fdo::Result<bool> {
        let (autostart_dir, launch_entry) = autostart_paths(&app_id)?;

        if !enable {
            return match tokio::fs::remove_file(&launch_entry).await {
                Ok(()) => Ok(false),
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
                Err(e) => Err(fdo::Error::FileNotFound(format!(
                    "{e}: ({})",
                    launch_entry.display()
                ))),
            };
        }

        tokio::fs::create_dir_all(&autostart_dir)
            .await
            .map_err(|e| fdo::Error::IOError(format!("{e}: ({})", autostart_dir.display())))?;

        let exec = desktop_exec(&commandline);
        let desktop_entry =
            desktop_entry(&app_id, &exec, flags, commandline_is_flatpak(&commandline));

        tokio::fs::write(&launch_entry, desktop_entry)
            .await
            .map_err(|e| fdo::Error::IOError(format!("{e}: ({})", launch_entry.display())))?;

        Ok(true)
    }

    async fn get_app_state(
        &self,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> fdo::Result<HashMap<String, AppStatus>> {
        get_app_state_impl(conn).await
    }

    #[zbus(signal)]
    pub async fn running_applications_changed(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

pub async fn route_background_dialog_responses(
    mut receiver: UnboundedReceiver<CopySelect>,
    pending_responses: PendingBackgroundResponses,
) {
    while let Some(selection) = receiver.next().await {
        let (handle, result) = match selection {
            CopySelect::BackgroundPermission { handle, result } => (handle, result),
            _ => continue,
        };

        let sender = {
            let mut pending_responses = pending_responses.lock().await;
            pending_responses.take_app_responses_by_handle(&handle)
        };

        if sender.is_empty() {
            tracing::debug!("Dropping stale background dialog response for {handle}");
        } else {
            for sender in sender {
                let _ = sender.send(result);
            }
        }
    }
}

fn validate_app_id(app_id: &str) -> fdo::Result<()> {
    if app_id.is_empty()
        || app_id.starts_with('.')
        || app_id.ends_with('.')
        || !app_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
    {
        return Err(fdo::Error::InvalidArgs(format!(
            "invalid application id: {app_id:?}"
        )));
    }

    Ok(())
}

fn autostart_paths(app_id: &str) -> fdo::Result<(PathBuf, PathBuf)> {
    validate_app_id(app_id)?;

    let autostart_dir = user_config_dir()?.join("autostart");
    let launch_entry = autostart_dir.join(format!("{app_id}.desktop"));
    Ok((autostart_dir, launch_entry))
}

fn user_config_dir() -> fdo::Result<PathBuf> {
    let config_home = XDG_CONFIG_HOME.clone().ok_or(fdo::Error::FileNotFound(
        "XDG_CONFIG_HOME and XDG_CONFIG_HOME are not set".into(),
    ))?;
    Ok(config_home)
}

fn desktop_entry(app_id: &str, exec: &str, flags: u32, flatpak: bool) -> String {
    let mut entry = String::from("[Desktop Entry]\n");

    push_desktop_entry_string(&mut entry, "Type", "Application");
    push_desktop_entry_string(&mut entry, "Name", app_id);
    push_desktop_entry_string(&mut entry, "Exec", exec);
    if flags & AUTOSTART_DBUS_ACTIVATABLE != 0 {
        entry.push_str("DBusActivatable=true\n");
    }
    if flatpak {
        push_desktop_entry_string(&mut entry, "X-Flatpak", app_id);
    }
    entry
}

fn push_desktop_entry_string(entry: &mut String, key: &str, value: &str) {
    entry.push_str(key);
    entry.push('=');
    push_desktop_entry_value(entry, value);
    entry.push('\n');
}

fn push_desktop_entry_value(entry: &mut String, value: &str) {
    for ch in value.chars() {
        match ch {
            '\\' => entry.push_str("\\\\"),
            '\n' => entry.push_str("\\n"),
            '\r' => entry.push_str("\\r"),
            '\t' => entry.push_str("\\t"),
            _ => entry.push(ch),
        }
    }
}

fn commandline_is_flatpak(commandline: &[String]) -> bool {
    commandline
        .first()
        .and_then(|command| std::path::Path::new(command).file_name())
        .is_some_and(|command| command == "flatpak")
}

fn desktop_exec(commandline: &[String]) -> String {
    commandline
        .iter()
        .map(|term| desktop_exec_arg(term))
        .collect::<Vec<_>>()
        .join(" ")
}

fn desktop_exec_arg(arg: &str) -> String {
    let mut needs_quotes = arg.is_empty();
    let mut escaped = String::with_capacity(arg.len());

    for ch in arg.chars() {
        if is_desktop_exec_reserved(ch) {
            needs_quotes = true;
        }

        match ch {
            '%' => escaped.push_str("%%"),
            '"' | '`' | '$' | '\\' => {
                escaped.push('\\');
                escaped.push(ch);
            }
            _ => escaped.push(ch),
        }
    }

    if needs_quotes {
        format!("\"{escaped}\"")
    } else {
        escaped
    }
}

fn is_desktop_exec_reserved(ch: char) -> bool {
    matches!(
        ch,
        ' ' | '\t'
            | '\n'
            | '"'
            | '\''
            | '\\'
            | '>'
            | '<'
            | '~'
            | '|'
            | '&'
            | ';'
            | '$'
            | '*'
            | '?'
            | '#'
            | '('
            | ')'
            | '`'
    )
}

fn merge_app_status(apps: &mut HashMap<String, AppStatus>, app_id: String, status: AppStatus) {
    apps.entry(app_id)
        .and_modify(|current| {
            if status > *current {
                *current = status;
            }
        })
        .or_insert(status);
}

fn normalized_app_token(app_id: &str) -> Option<String> {
    let token = app_id.rsplit('.').next().unwrap_or(app_id);
    let normalized = token
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(|character| character.to_lowercase())
        .collect::<String>();

    (!normalized.is_empty()).then_some(normalized)
}

fn app_token_index(app_ids: &[String]) -> HashMap<String, Option<String>> {
    let mut index = HashMap::new();

    for app_id in app_ids {
        let Some(token) = normalized_app_token(app_id) else {
            continue;
        };

        match index.entry(token) {
            Entry::Vacant(entry) => {
                entry.insert(Some(app_id.clone()));
            }
            Entry::Occupied(mut entry) if entry.get().as_deref() != Some(app_id.as_str()) => {
                entry.insert(None);
            }
            Entry::Occupied(_) => {}
        }
    }

    index
}

fn fuzzy_systemd_app_id<'a>(
    wayland_app_id: &str,
    systemd_app_tokens: &'a HashMap<String, Option<String>>,
) -> Option<&'a str> {
    let token = normalized_app_token(wayland_app_id)?;
    systemd_app_tokens.get(&token)?.as_deref()
}

pub(crate) async fn get_app_state_impl(
    conn: &zbus::Connection,
) -> fdo::Result<HashMap<String, AppStatus>> {
    let mut apps = HashMap::new();
    let mut systemd_app_ids = Vec::new();
    let mut systemd_error = None;

    match Systemd1Proxy::new(conn).await {
        Ok(proxy) => match proxy.list_units().await {
            Ok(units) => {
                for unit in units {
                    if let Some(app_id) = unit.app_id() {
                        let app_id = app_id.to_owned();
                        merge_app_status(&mut apps, app_id.clone(), AppStatus::Background);
                        systemd_app_ids.push(app_id);
                    }
                }
            }
            Err(e) => {
                tracing::warn!("Cannot list systemd app scopes: {e}");
                systemd_error = Some(e.to_string());
            }
        },
        Err(e) => {
            tracing::warn!("Cannot connect to systemd manager: {e}");
            systemd_error = Some(e.to_string());
        }
    }

    let systemd_app_tokens = app_token_index(&systemd_app_ids);

    match WayshotConnection::new() {
        Ok(mut wayshot_connection) => {
            if let Err(e) = wayshot_connection.refresh_toplevels() {
                tracing::warn!("Cannot refresh Wayland toplevels: {e:?}");
            } else {
                for top_level in wayshot_connection.get_all_toplevels() {
                    if top_level.app_id.is_empty() {
                        continue;
                    }

                    // ext-foreign-toplevel-list reports toplevel presence, but
                    // not activation/focus state. libwayshot's active flag is
                    // therefore not meaningful here.
                    if apps.contains_key(&top_level.app_id) {
                        merge_app_status(&mut apps, top_level.app_id.clone(), AppStatus::Running);
                    } else if let Some(systemd_app_id) =
                        fuzzy_systemd_app_id(&top_level.app_id, &systemd_app_tokens)
                    {
                        tracing::debug!(
                            "Treating Wayland app id {} as systemd app id {}",
                            top_level.app_id,
                            systemd_app_id
                        );
                        merge_app_status(&mut apps, systemd_app_id.to_owned(), AppStatus::Running);
                    } else {
                        merge_app_status(&mut apps, top_level.app_id.clone(), AppStatus::Running);
                    }
                }
            }
        }
        Err(e) => {
            tracing::warn!("Cannot get Wayland toplevels: {e:?}");
        }
    }

    if let Some(systemd_error) = systemd_error {
        tracing::debug!("Returning app state without systemd scope data: {systemd_error}");
    }

    tracing::debug!("GetAppState is returning {} open apps", apps.len());
    Ok(apps)
}

pub fn unit_name_from_systemd_signal(message: &zbus::Message) -> Option<String> {
    message
        .body()
        .deserialize::<(String, zbus::zvariant::OwnedObjectPath)>()
        .ok()
        .map(|(unit_name, _)| unit_name)
}
