mod config;
use tokio::sync::Mutex;
use zbus::{fdo, interface, object_server::SignalEmitter};

use zbus::zvariant::{DeserializeDict, OwnedValue, SerializeDict, Type, Value};

const DEFAULT_COLOR: u32 = 0;
const DARK_COLOR: u32 = 1;
const LIGHT_COLOR: u32 = 2;

const APPEARANCE: &str = "org.freedesktop.appearance";
const COLOR_SCHEME: &str = "color-scheme";
const ACCENT_COLOR: &str = "accent-color";

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::LazyLock;

pub use self::config::SettingsConfig;

pub static SETTING_CONFIG: LazyLock<Arc<Mutex<SettingsConfig>>> =
    LazyLock::new(|| Arc::new(Mutex::new(SettingsConfig::config_from_file())));

#[derive(DeserializeDict, SerializeDict, Clone, Copy, PartialEq, Type, OwnedValue, Value)]
pub struct AccentColor {
    red: f64,
    green: f64,
    blue: f64,
}

impl AccentColor {
    pub fn new(rgb: [f64; 3]) -> Self {
        Self {
            red: rgb[0],
            green: rgb[1],
            blue: rgb[2],
        }
    }
}

#[derive(Debug)]
pub struct SettingsBackend;

#[interface(name = "org.freedesktop.impl.portal.Settings")]
impl SettingsBackend {
    #[zbus(property, name = "version")]
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
            return Ok(AccentColor::new(config.get_accent_color())
                .try_into()
                .unwrap());
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
            OwnedValue::try_from(AccentColor::new(config.get_accent_color())).unwrap(),
        );
        Ok(output.into())
    }

    #[zbus(signal)]
    pub async fn setting_changed(
        ctxt: &SignalEmitter<'_>,
        namespace: String,
        key: String,
        value: OwnedValue,
    ) -> zbus::Result<()>;
    // add code here
}
