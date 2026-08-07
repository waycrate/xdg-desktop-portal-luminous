use std::{
    collections::HashMap,
    io,
    os::fd::{AsFd, OwnedFd},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
};

use calloop::{
    EventLoop,
    channel::{self, Channel, SyncSender},
};
use calloop_wayland_source::WaylandSource;
use os_pipe::pipe;
use sctk::registry::{ProvidesRegistryState, RegistryState};
use tokio::sync::{mpsc::Sender as SignalSender, oneshot::Sender as OneSender, watch};
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle, backend::ObjectId, delegate_noop,
    event_created_child, globals::registry_queue_init, protocol::wl_seat::WlSeat,
};
use wayland_protocols::ext::data_control::v1::client::{
    ext_data_control_device_v1::{self, ExtDataControlDeviceV1},
    ext_data_control_manager_v1::ExtDataControlManagerV1,
    ext_data_control_offer_v1::{self, ExtDataControlOfferV1},
    ext_data_control_source_v1::{self, ExtDataControlSourceV1},
};
use zbus::zvariant::OwnedObjectPath;

static CLIPBOARD_SERIAL: AtomicU32 = AtomicU32::new(1);

// caps fds pinned by clients that never call SelectionWriteDone
const MAX_PENDING_TRANSFERS: usize = 16;
// bounds queued commands against a stalled worker; try_send surfaces Full as a D-Bus error
const REQUEST_QUEUE_DEPTH: usize = 64;

pub(super) struct TransferEvent {
    pub(super) mime_type: String,
    pub(super) serial: u32,
}

#[derive(Clone)]
pub(super) struct OwnerState {
    pub(super) mime_types: Vec<String>,
    pub(super) session_is_owner: bool,
}

pub(super) enum ClipboardRequest {
    SetSelection {
        mime_types: Vec<String>,
    },
    Write {
        serial: u32,
        sender: OneSender<Result<OwnedFd, String>>,
    },
    WriteDone {
        serial: u32,
        success: bool,
        sender: OneSender<Result<(), String>>,
    },
    Read {
        mime_type: String,
        sender: OneSender<Result<OwnedFd, String>>,
    },
    Stop,
}

pub(super) struct ClipboardThread {
    pub(super) sender: SyncSender<ClipboardRequest>,
    pub(super) shutdown: Arc<AtomicBool>,
    pub(super) exit_rx: watch::Receiver<()>,
}

impl ClipboardThread {
    pub(super) fn spawn(
        transfer_tx: SignalSender<TransferEvent>,
        owner_tx: watch::Sender<Option<OwnerState>>,
        ready_tx: tokio::sync::oneshot::Sender<Result<(), String>>,
        session_path: OwnedObjectPath,
    ) -> io::Result<Self> {
        let (sender, receiver) = channel::sync_channel(REQUEST_QUEUE_DEPTH);
        let shutdown = Arc::new(AtomicBool::new(false));
        let worker_shutdown = shutdown.clone();
        let (exit_tx, exit_rx) = watch::channel(());
        let log_path = session_path.clone();

        std::thread::Builder::new()
            .name("luminous-clipboard".to_owned())
            .spawn(move || {
                if let Err(error) = clipboard_loop(
                    transfer_tx,
                    owner_tx,
                    ready_tx,
                    exit_tx,
                    receiver,
                    worker_shutdown,
                    session_path,
                ) {
                    tracing::error!(%log_path, %error, "clipboard worker failed");
                }
                tracing::info!(%log_path, "clipboard worker exited");
            })?;

        Ok(Self {
            sender,
            shutdown,
            exit_rx,
        })
    }
}

impl Drop for ClipboardThread {
    fn drop(&mut self) {
        // NOTE: never block here (runs inside Session.Close); the flag alone stops the worker
        self.shutdown.store(true, Ordering::SeqCst);
        let _ = self.sender.try_send(ClipboardRequest::Stop);
    }
}

struct ClipboardWl {
    registry_state: RegistryState,
    data_manager: ExtDataControlManagerV1,
    device: ExtDataControlDeviceV1,
    qh: QueueHandle<Self>,
    connection: Connection,
    transfer_tx: SignalSender<TransferEvent>,
    owner_tx: watch::Sender<Option<OwnerState>>,
    loop_signal: calloop::LoopSignal,
    defunct: bool,
    shutdown: Arc<AtomicBool>,
    pending_offers: HashMap<ObjectId, Vec<String>>,
    current_selection: Option<ExtDataControlOfferV1>,
    current_mime_types: Vec<String>,
    primary_selection: Option<ExtDataControlOfferV1>,
    own_source: Option<ExtDataControlSourceV1>,
    is_owner: bool,
    pending_writes: HashMap<u32, OwnedFd>,
}

impl Dispatch<ExtDataControlDeviceV1, ()> for ClipboardWl {
    fn event(
        state: &mut Self,
        _proxy: &ExtDataControlDeviceV1,
        event: ext_data_control_device_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        match event {
            ext_data_control_device_v1::Event::DataOffer { id } => {
                state.pending_offers.insert(id.id(), Vec::new());
            }
            ext_data_control_device_v1::Event::Selection { id } => {
                if let Some(old) = state.current_selection.take() {
                    state.pending_offers.remove(&old.id());
                    old.destroy();
                }
                let mime_types = match &id {
                    Some(offer) => state.pending_offers.remove(&offer.id()).unwrap_or_default(),
                    None => Vec::new(),
                };
                state.current_mime_types = mime_types.clone();
                state.current_selection = id;
                state.owner_tx.send_replace(Some(OwnerState {
                    mime_types,
                    session_is_owner: state.is_owner,
                }));
            }
            ext_data_control_device_v1::Event::PrimarySelection { id } => {
                if let Some(offer) = &id {
                    state.pending_offers.remove(&offer.id());
                }
                state.reset_primary_offer(id);
            }
            ext_data_control_device_v1::Event::Finished => {
                tracing::error!("ext_data_control device finished; stopping clipboard worker");
                state.defunct = true;
                state.reset_offer();
                state.loop_signal.stop();
            }
            _ => unreachable!(),
        }
    }

    event_created_child!(ClipboardWl, ExtDataControlDeviceV1, [
        ext_data_control_device_v1::EVT_DATA_OFFER_OPCODE => (ExtDataControlOfferV1, ())
    ]);
}

impl Dispatch<ExtDataControlSourceV1, ()> for ClipboardWl {
    fn event(
        state: &mut Self,
        proxy: &ExtDataControlSourceV1,
        event: ext_data_control_source_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        match event {
            ext_data_control_source_v1::Event::Send { mime_type, fd } => {
                if state.pending_writes.len() >= MAX_PENDING_TRANSFERS {
                    tracing::warn!(
                        "pending clipboard transfers at capacity; rejecting paste request"
                    );
                    return;
                }
                let serial = CLIPBOARD_SERIAL.fetch_add(1, Ordering::Relaxed);
                state.pending_writes.insert(serial, fd);
                if state
                    .transfer_tx
                    .try_send(TransferEvent { mime_type, serial })
                    .is_err()
                {
                    state.pending_writes.remove(&serial);
                }
            }
            ext_data_control_source_v1::Event::Cancelled => {
                if state.own_source.as_ref().map(Proxy::id) == Some(proxy.id()) {
                    state.own_source = None;
                    state.is_owner = false;
                    state.owner_tx.send_replace(Some(OwnerState {
                        mime_types: state.current_mime_types.clone(),
                        session_is_owner: false,
                    }));
                }
                proxy.destroy();
            }
            _ => unreachable!(),
        }
    }
}

impl Dispatch<ExtDataControlOfferV1, ()> for ClipboardWl {
    fn event(
        state: &mut Self,
        proxy: &ExtDataControlOfferV1,
        event: ext_data_control_offer_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        if let ext_data_control_offer_v1::Event::Offer { mime_type } = event
            && let Some(mimes) = state.pending_offers.get_mut(&proxy.id())
        {
            mimes.push(mime_type);
        }
    }
}

impl ProvidesRegistryState for ClipboardWl {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }

    sctk::registry_handlers![];
}

delegate_noop!(ClipboardWl: ignore WlSeat);
delegate_noop!(ClipboardWl: ignore ExtDataControlManagerV1);
sctk::delegate_registry!(ClipboardWl);

impl ClipboardWl {
    fn reset_offer(&mut self) {
        if let Some(offer) = self.current_selection.take() {
            offer.destroy();
        }
        self.current_mime_types.clear();
    }

    fn reset_primary_offer(&mut self, id: Option<ExtDataControlOfferV1>) {
        if let Some(offer) = self.primary_selection.take() {
            offer.destroy();
        }
        self.primary_selection = id;
    }

    fn set_selection(&mut self, mime_types: Vec<String>) {
        if mime_types.is_empty() {
            self.device.set_selection(None);
            self.own_source = None;
            self.is_owner = false;
            self.owner_tx.send_replace(Some(OwnerState {
                mime_types: Vec::new(),
                session_is_owner: false,
            }));
        } else {
            let source = self.data_manager.create_data_source(&self.qh, ());
            for mime_type in &mime_types {
                source.offer(mime_type.clone());
            }
            self.device.set_selection(Some(&source));
            self.own_source = Some(source);
            self.is_owner = true;
        }
        let _ = self.connection.flush();
    }

    fn read_selection(&mut self, mime_type: &str) -> Result<OwnedFd, String> {
        let (read, write) = pipe().map_err(|error| format!("pipe failed: {error}"))?;
        let offer = self.current_selection.as_ref().filter(|_| {
            self.current_mime_types
                .iter()
                .any(|offered| offered == mime_type)
        });
        if let Some(offer) = offer {
            offer.receive(mime_type.to_owned(), write.as_fd());
            let _ = self.connection.flush();
        }
        drop(write);
        Ok(read.into())
    }
}

fn handle_request(
    app_state: &mut ClipboardWl,
    signal: &calloop::LoopSignal,
    message: ClipboardRequest,
) {
    if app_state.shutdown.load(Ordering::SeqCst) || app_state.defunct {
        signal.stop();
        return;
    }

    match message {
        ClipboardRequest::SetSelection { mime_types } => {
            app_state.set_selection(mime_types);
        }
        ClipboardRequest::Read { mime_type, sender } => {
            let result = app_state.read_selection(&mime_type);
            let _ = sender.send(result);
        }
        ClipboardRequest::Write { serial, sender } => {
            let result = match app_state.pending_writes.get(&serial) {
                Some(fd) => fd
                    .try_clone()
                    .map_err(|error| format!("dup failed: {error}")),
                None => Err(format!("no pending transfer with serial {serial}")),
            };
            let _ = sender.send(result);
        }
        ClipboardRequest::WriteDone {
            serial,
            success,
            sender,
        } => {
            tracing::debug!(serial, success, "selection write done");
            let result = match app_state.pending_writes.remove(&serial) {
                Some(_) => Ok(()),
                None => Err(format!("no pending transfer with serial {serial}")),
            };
            let _ = sender.send(result);
        }
        ClipboardRequest::Stop => {
            app_state.defunct = true;
            signal.stop();
        }
    }
}

fn clipboard_loop(
    transfer_tx: SignalSender<TransferEvent>,
    owner_tx: watch::Sender<Option<OwnerState>>,
    ready_tx: tokio::sync::oneshot::Sender<Result<(), String>>,
    exit_tx: watch::Sender<()>,
    receiver: Channel<ClipboardRequest>,
    shutdown: Arc<AtomicBool>,
    session_path: OwnedObjectPath,
) -> anyhow::Result<()> {
    tracing::debug!(%session_path, "initializing clipboard worker");

    macro_rules! ready_try {
        ($expression:expr) => {
            match $expression {
                Ok(value) => value,
                Err(error) => {
                    let message = error.to_string();
                    let _ = ready_tx.send(Err(message.clone()));
                    return Err(anyhow::anyhow!(message));
                }
            }
        };
    }

    let connection = ready_try!(Connection::connect_to_env());
    let (globals, mut event_queue) = ready_try!(registry_queue_init::<ClipboardWl>(&connection));
    let qh = event_queue.handle();
    let seat = ready_try!(globals.bind::<WlSeat, _, _>(&qh, 1..=1, ()));
    let data_manager = ready_try!(globals.bind::<ExtDataControlManagerV1, _, _>(&qh, 1..=1, ()));
    let device = data_manager.get_data_device(&seat, &qh, ());
    let mut event_loop: EventLoop<ClipboardWl> = ready_try!(EventLoop::try_new());
    let loop_signal = event_loop.get_signal();
    let mut clipboard = ClipboardWl {
        registry_state: RegistryState::new(&globals),
        data_manager,
        device,
        qh,
        connection: connection.clone(),
        transfer_tx,
        owner_tx,
        loop_signal,
        defunct: false,
        shutdown,
        pending_offers: HashMap::new(),
        current_selection: None,
        current_mime_types: Vec::new(),
        primary_selection: None,
        own_source: None,
        is_owner: false,
        pending_writes: HashMap::new(),
    };
    // Rebinding after `clipboard` makes the exit signal drop first during unwinding.
    let exit_tx = exit_tx;

    ready_try!(event_queue.roundtrip(&mut clipboard));
    if clipboard.defunct {
        let message = "ext_data_control device finished during initialization".to_owned();
        let _ = ready_tx.send(Err(message.clone()));
        return Err(anyhow::anyhow!(message));
    }

    ready_try!(WaylandSource::new(connection.clone(), event_queue).insert(event_loop.handle()));

    let request_signal = event_loop.get_signal();
    ready_try!(
        event_loop
            .handle()
            .insert_source(receiver, move |event, _, app_state| {
                let message = match event {
                    channel::Event::Msg(message) => message,
                    channel::Event::Closed => {
                        request_signal.stop();
                        return;
                    }
                };
                handle_request(app_state, &request_signal, message);
            })
    );

    let _ = ready_tx.send(Ok(()));

    let result = event_loop.run(
        std::time::Duration::from_millis(20),
        &mut clipboard,
        |state| {
            if state.shutdown.load(Ordering::SeqCst) || state.defunct {
                state.loop_signal.stop();
            }
        },
    );
    drop(exit_tx);
    result?;
    Ok(())
}
