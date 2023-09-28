mod config;
use tokio::sync::Mutex;
use zbus::{dbus_interface, fdo, SignalContext};

use zbus::zvariant::{Array, DeserializeDict, OwnedValue, SerializeDict, Signature, Type};

const DEFAULT_COLOR: u32 = 0;
const DARK_COLOR: u32 = 1;
const LIGHT_COLOR: u32 = 2;

const APPEARANCE: &str = "org.freedesktop.appearance";
const COLOR_SCHEME: &str = "color-scheme";
const ACCENT_COLOR: &str = "accent-color";

use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::sync::Arc;

pub use self::config::SettingsConfig;

pub static SETTING_CONFIG: Lazy<Arc<Mutex<SettingsConfig>>> =
    Lazy::new(|| Arc::new(Mutex::new(SettingsConfig::config_from_file())));

#[derive(DeserializeDict, SerializeDict, Clone, Copy, PartialEq, Type)]
#[zvariant(signature = "dict")]
pub struct AccentColor {
    pub color: [f64; 3],
}

impl Into<OwnedValue> for AccentColor {
    fn into(self) -> OwnedValue {
        let arraysignature = Signature::try_from("d").unwrap();
        let mut array = Array::new(arraysignature);
        for col in self.color {
            array.append(col.into()).unwrap();
        }
        OwnedValue::from(array)
    }
}

#[derive(Debug)]
pub struct SettingsBackend;

#[dbus_interface(name = "org.freedesktop.impl.portal.Settings")]
impl SettingsBackend {
    #[dbus_interface(property, name = "version")]
    fn version(&self) -> u32 {
        1
    }

    async fn read(&self, namespace: String, key: String) -> fdo::Result<OwnedValue> {
        if namespace != APPEARANCE {
            return Err(zbus::fdo::Error::Failed("No such namespace".to_string()));
        }
        let config = SETTING_CONFIG.lock().await;
        if key == COLOR_SCHEME {
            return Ok(OwnedValue::from(config.get_color_scheme()));
        }
        if key == ACCENT_COLOR {
            return Ok(AccentColor {
                color: config.get_accent_color(),
            }
            .into());
        }
        Err(zbus::fdo::Error::Failed("No such namespace".to_string()))
    }

    async fn read_all(&self, namespace: String) -> fdo::Result<OwnedValue> {
        if namespace != APPEARANCE {
            return Err(zbus::fdo::Error::Failed("No such namespace".to_string()));
        }
        let mut output = HashMap::<String, OwnedValue>::new();
        let config = SETTING_CONFIG.lock().await;
        output.insert(COLOR_SCHEME.to_string(), config.get_color_scheme().into());
        output.insert(
            ACCENT_COLOR.to_string(),
            AccentColor {
                color: config.get_accent_color(),
            }
            .into(),
        );
        Ok(output.into())
    }

    #[dbus_interface(signal)]
    pub async fn setting_changed(
        ctxt: &SignalContext<'_>,
        namespace: String,
        key: String,
        value: OwnedValue,
    ) -> zbus::Result<()>;
    // add code here
}
