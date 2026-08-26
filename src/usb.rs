use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::{SinkExt, StreamExt, channel::mpsc::Receiver};
use serde::Deserialize;
use tokio::sync::oneshot;
use zbus::{
    fdo, interface,
    zvariant::{Dict, ObjectPath, OwnedObjectPath, OwnedValue, Type, Value, as_value},
};

use crate::dialog::{Message, UsbDeviceEntry};
use crate::request::{RequestCloseAction, RequestInterface};
use crate::{PortalResponse, settings::SETTING_CONFIG};

/// One requested USB device: identifier plus the device information and
/// access option dictionaries sent by the portal frontend.
#[derive(Debug, Type, Deserialize)]
#[zvariant(signature = "(sa{sv}a{sv})")]
struct UsbDeviceData {
    id: String,
    info: DeviceInfo,
    access: AccessOptions,
}

/// The `info` dictionary: keys documented in
/// `org.freedesktop.portal.Usb.EnumerateDevices`.
#[derive(Debug, Default, Type, Deserialize)]
#[zvariant(signature = "dict")]
struct DeviceInfo {
    /// Raw udev properties forwarded by the portal (`ID_VENDOR_ID`,
    /// `ID_MODEL_FROM_DATABASE`, …).
    #[serde(
        rename = "properties",
        with = "as_value::optional",
        skip_serializing_if = "Option::is_none",
        default
    )]
    properties: Option<HashMap<String, OwnedValue>>,
}

/// The `access options` dictionary: `writable` (b), default false.
#[derive(Debug, Default, Type, Deserialize)]
#[zvariant(signature = "dict")]
struct AccessOptions {
    #[serde(rename = "writable", with = "as_value", default)]
    writable: bool,
}

/// Reads a udev property that arrived variant-wrapped.
fn prop_str(value: &OwnedValue) -> Option<String> {
    match &**value {
        Value::Str(string) => Some(string.to_string()),
        _ => None,
    }
}

/// Converts accepted access options back into their wire representation.
fn access_to_wire(access: &AccessOptions) -> OwnedValue {
    let mut dict = Dict::new(
        &zbus::zvariant::Signature::Str,
        &zbus::zvariant::Signature::Variant,
    );
    if access.writable {
        // Only mention writable when actually requested, so the echo stays
        // verbatim for read-only acquires.
        dict.add("writable".to_string(), Value::Bool(true)).unwrap();
    }
    OwnedValue::try_from(Value::Dict(dict)).expect("dict converts to owned value")
}

/// Results of a successful acquire: identifier plus echoed access options.
#[derive(Type, Debug, serde::Serialize)]
#[zvariant(signature = "dict")]
struct AcquireDevicesResult {
    #[serde(with = "as_value")]
    devices: Vec<(String, OwnedValue)>,
}

/// Decodes hex escapes in udev strings (`\x20` for space and friends).
fn parse_udev_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.char_indices();
    while let Some((index, c)) = chars.next() {
        if c == '\\'
            && s.as_bytes().get(index + 1) == Some(&b'x')
            && let Some(hex) = s.get(index + 2..index + 4)
            && let Ok(byte) = u8::from_str_radix(hex, 16)
        {
            if byte != 0 {
                out.push(byte as char);
            }
            // Skip `x` and both hex digits.
            chars.next();
            chars.next();
            chars.next();
            continue;
        }
        out.push(c);
    }
    out
}

/// Human readable label for the dialog, best effort from udev properties.
fn device_label(id: &str, info: &DeviceInfo, access: &AccessOptions) -> String {
    let Some(props) = &info.properties else {
        return id.to_string();
    };
    let prop = |keys: &[&str]| {
        keys.iter().find_map(|key| {
            let value = props.get(*key)?;
            Some(parse_udev_string(&prop_str(value)?))
        })
    };

    let vendor = prop(&[
        "ID_VENDOR_FROM_DATABASE",
        "ID_VENDOR",
        "ID_VENDOR_ENC",
        "ID_VENDOR_ID",
    ]);
    let model = prop(&[
        "ID_MODEL_FROM_DATABASE",
        "ID_MODEL",
        "ID_MODEL_ENC",
        "ID_MODEL_ID",
    ]);
    let serial = props
        .get("ID_SERIAL_SHORT")
        .and_then(prop_str)
        .filter(|serial| !serial.is_empty());

    let mut label = match (&model, &vendor) {
        (Some(model), Some(vendor)) => format!("{vendor} {model}"),
        (Some(model), None) => model.clone(),
        (None, Some(vendor)) => vendor.clone(),
        // ID_SERIAL is always set by udev's usb_id builtin: `vendor_model`,
        // suffixed with `_serial` when the device exposes one (synthetic
        // `046d_c52b` when descriptors are missing) — reads better than
        // the opaque portal id.
        (None, None) => prop(&["ID_SERIAL"]).unwrap_or_else(|| id.to_string()),
    };
    if let Some(serial) = serial {
        label.push_str(&format!(" SN:{serial}"));
    }

    // udev's usb_id builtin stores whitespace and characters outside its
    // devnode allowlist as underscores (`ID_MODEL=USB_Receiver`); render
    // them as single spaces for display.
    let label = label
        .split(['_', ' '])
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");

    if writable_requested(access) {
        format!("{label} (read-write)")
    } else {
        format!("{label} (read only)")
    }
}

fn writable_requested(access: &AccessOptions) -> bool {
    access.writable
}

/// Slot for the single in-flight acquire request. The dialog routes at most one
/// selection back through [`route_responses`]; whoever holds the slot receives it.
pub type ActiveClaim = Arc<Mutex<Option<oneshot::Sender<crate::dialog::CopySelect>>>>;

pub struct UsbBackend {
    pub sender: futures::channel::mpsc::Sender<Message>,
    pub active_claim: ActiveClaim,
}

/// Forwards the next dialog selection to the pending acquire request, if any.
///
/// Owning the receiver here keeps `acquire_devices` on `&self`, so a parked
/// request can never block other D-Bus dispatch on the desktop object. The
/// caller spawns this task once and shares `active_claim` with the backend.
pub async fn route_responses(
    mut receiver: Receiver<crate::dialog::CopySelect>,
    active_claim: ActiveClaim,
) {
    while let Some(select) = receiver.next().await {
        tracing::info!("USB router forwarding selection: {select:?}");
        let claim = active_claim
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        match claim {
            Some(claim) => {
                let _ = claim.send(select);
            }
            None => tracing::info!("USB router dropped selection: no pending request"),
        }
    }
}

#[interface(name = "org.freedesktop.impl.portal.Usb")]
impl UsbBackend {
    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        1
    }

    #[allow(clippy::too_many_arguments)]
    async fn acquire_devices(
        &self,
        handle: ObjectPath<'_>,
        _parent_window: String,
        app_id: String,
        devices: Vec<UsbDeviceData>,
        _options: HashMap<String, OwnedValue>,
        #[zbus(object_server)] server: &zbus::ObjectServer,
    ) -> fdo::Result<PortalResponse<AcquireDevicesResult>> {
        tracing::info!(
            "USB AcquireDevices: path: {}, appid: {}, devices: {}",
            handle.as_str(),
            app_id,
            devices.len()
        );

        // Auto approve everything when the user disabled the permission check.
        if !SETTING_CONFIG.lock().await.usb_permission_check {
            return Ok(PortalResponse::Success(AcquireDevicesResult {
                devices: devices
                    .into_iter()
                    .map(|device| (device.id, access_to_wire(&device.access)))
                    .collect(),
            }));
        }

        // Only one USB dialog at a time; further concurrent requests are
        // rejected instead of queueing behind user interaction.
        let (response_sender, response_receiver) = oneshot::channel();
        {
            let mut claim = self
                .active_claim
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if claim.is_some() {
                return Ok(PortalResponse::Cancelled);
            }
            *claim = Some(response_sender);
        }

        let entries: Vec<UsbDeviceEntry> = devices
            .iter()
            .map(|device| UsbDeviceEntry {
                id: device.id.clone(),
                label: device_label(&device.id, &device.info, &device.access),
                checked: true,
            })
            .collect();

        let handle_path: OwnedObjectPath = handle.into();
        let (cancel_sender, cancel_receiver) = oneshot::channel();
        server
            .at(
                handle_path.clone(),
                RequestInterface {
                    handle_path: handle_path.clone(),
                    close_action: Some(RequestCloseAction {
                        cancel_sender: Arc::new(Mutex::new(Some(cancel_sender))),
                        ui_sender: self.sender.clone(),
                        close_message: Some(Message::CloseUsbPrompt),
                    }),
                },
            )
            .await?;

        if let Err(e) = self
            .sender
            .clone()
            .send(Message::UsbAcquireDialog { app_id, entries })
            .await
        {
            self.active_claim
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .take();
            let _ = server
                .remove::<RequestInterface, &OwnedObjectPath>(&handle_path)
                .await;
            return Err(zbus::Error::Failure(e.to_string()).into());
        }

        let granted = tokio::select! {
            biased;
            _ = cancel_receiver => {
                tracing::info!("USB acquire cancelled via Request.Close");
                None
            }
            select = response_receiver => {
                tracing::info!("USB acquire got dialog response: {select:?}");
                match select {
                    Ok(crate::dialog::CopySelect::Usb(ids)) if !ids.is_empty() => Some(ids),
                    _ => None,
                }
            }
        };

        // Release the claim so later requests can prompt again.
        self.active_claim
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();

        let _ = server
            .remove::<RequestInterface, &OwnedObjectPath>(&handle_path)
            .await;

        let Some(granted) = granted else {
            return Ok(PortalResponse::Cancelled);
        };

        // reserve time to let dialog disappear
        tokio::time::sleep(Duration::from_secs(1)).await;

        Ok(PortalResponse::Success(AcquireDevicesResult {
            devices: devices
                .into_iter()
                .filter(|device| granted.contains(&device.id))
                .map(|device| (device.id, access_to_wire(&device.access)))
                .collect(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a `a{sv}` dictionary value from `(key, value)` pairs.
    fn wire_dict_value<const N: usize>(entries: [(&str, Value<'static>); N]) -> Value<'static> {
        let mut dict = Dict::new(
            &zbus::zvariant::Signature::Str,
            &zbus::zvariant::Signature::Variant,
        );
        for (key, value) in entries {
            dict.add(key.to_string(), value).unwrap();
        }
        Value::Dict(dict)
    }

    /// udev properties as they arrive: variant-wrapped strings.
    fn owned_props<const N: usize>(entries: [(&str, &str); N]) -> HashMap<String, OwnedValue> {
        entries
            .into_iter()
            .map(|(key, value)| {
                (
                    key.to_string(),
                    OwnedValue::try_from(Value::from(value))
                        .expect("string converts to owned value"),
                )
            })
            .collect()
    }

    #[test]
    fn parse_udev_space_escapes() {
        assert_eq!(parse_udev_string("Logitech\\x20Mouse"), "Logitech Mouse");
        assert_eq!(parse_udev_string("A\\x20B\\x20C"), "A B C");
    }

    #[test]
    fn parse_udev_without_escapes() {
        assert_eq!(parse_udev_string("SimpleDevice"), "SimpleDevice");
        assert_eq!(parse_udev_string(""), "");
    }

    #[test]
    fn parse_udev_keeps_invalid_escapes() {
        // Not a valid hex escape: must stay verbatim.
        assert_eq!(parse_udev_string("\\xzz"), "\\xzz");
        assert_eq!(parse_udev_string("trailing\\x2"), "trailing\\x2");
    }

    #[test]
    fn device_label_prefers_database_names() {
        let info = DeviceInfo {
            properties: Some(owned_props([
                ("ID_VENDOR_FROM_DATABASE", "Logitech"),
                ("ID_MODEL_FROM_DATABASE", "G\\x20502"),
            ])),
        };
        let access = AccessOptions { writable: true };
        assert_eq!(
            device_label("dev-0", &info, &access),
            "Logitech G 502 (read-write)"
        );
    }

    #[test]
    fn device_label_falls_back_to_serial_then_id() {
        let with_serial = DeviceInfo {
            properties: Some(owned_props([("ID_SERIAL", "Logitech_USB_Receiver")])),
        };
        assert_eq!(
            device_label("dev-0", &with_serial, &AccessOptions::default()),
            "Logitech USB Receiver (read only)"
        );

        assert_eq!(
            device_label("dev-1", &DeviceInfo::default(), &AccessOptions::default()),
            "dev-1"
        );
    }

    #[test]
    fn device_label_collapses_prettified_underscores() {
        // `A & B` becomes `A___B` through udev's whitespace/char replacement.
        let info = DeviceInfo {
            properties: Some(owned_props([
                ("ID_VENDOR", "Acme___Corp"),
                ("ID_MODEL", "Ultra_Drive"),
            ])),
        };
        assert_eq!(
            device_label("dev-0", &info, &AccessOptions::default()),
            "Acme Corp Ultra Drive (read only)"
        );
    }

    /// The frontend sends every dictionary value variant-wrapped; build the
    /// frame the same way and check the typed fields deserialize through it.
    #[test]
    fn input_deserialization_unwraps_variants() {
        // Same shape as the real message body: `sa{sv}a{sv}` where every
        // dictionary entry value is variant-framed.
        let info: HashMap<String, Value<'static>> = HashMap::from([(
            "properties".to_string(),
            wire_dict_value([("ID_SERIAL", Value::from("Logitech_USB_Receiver"))]),
        )]);
        let access: HashMap<String, Value<'static>> =
            HashMap::from([("writable".to_string(), Value::Bool(true))]);
        let frame = ("dev-0".to_string(), info, access);
        let context =
            zbus::zvariant::serialized::Context::new_dbus(zbus::zvariant::Endian::Little, 0);
        let encoded = zbus::zvariant::to_bytes(context, &frame).unwrap();
        let decoded: UsbDeviceData = encoded.deserialize().unwrap().0;

        assert_eq!(decoded.id, "dev-0");
        let props = decoded
            .info
            .properties
            .as_ref()
            .expect("properties present");
        assert_eq!(
            props.get("ID_SERIAL").and_then(prop_str).as_deref(),
            Some("Logitech_USB_Receiver")
        );
        assert!(decoded.access.writable);

        // Absent key falls back to the documented default (false).
        let frame = (
            "dev-1".to_string(),
            HashMap::<String, Value<'static>>::new(),
            HashMap::<String, Value<'static>>::new(),
        );
        let encoded = zbus::zvariant::to_bytes(context, &frame).unwrap();
        let decoded: UsbDeviceData = encoded.deserialize().unwrap().0;
        assert!(!decoded.access.writable);
    }

    #[test]
    fn acquire_reply_serialization_roundtrip() {
        use crate::PortalResponse;

        let response: PortalResponse<AcquireDevicesResult> =
            PortalResponse::Success(AcquireDevicesResult {
                devices: vec![(
                    "dev-0".to_string(),
                    access_to_wire(&AccessOptions { writable: true }),
                )],
            });

        let context =
            zbus::zvariant::serialized::Context::new_dbus(zbus::zvariant::Endian::Little, 0);
        let encoded = zbus::zvariant::to_bytes(context, &response).unwrap();
        let decoded: (u32, HashMap<String, zbus::zvariant::Value<'_>>) =
            encoded.deserialize().unwrap().0;
        assert_eq!(decoded.0, 0);
        assert_eq!(decoded.1.len(), 1);
    }
}
