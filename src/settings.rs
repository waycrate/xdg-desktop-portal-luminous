mod config;
use zbus::{dbus_interface, fdo, SignalContext};

use zbus::zvariant::{Array, DeserializeDict, OwnedValue, SerializeDict, Signature, Type};

const DEFAULT_COLOR: u32 = 0;
const DARK_COLOR: u32 = 1;
const LIGHT_COLOR: u32 = 2;

const APPEARANCE: &str = "org.freedesktop.appearance";
const COLOR_SCHEME: &str = "color-scheme";
const ACCENT_COLOR: &str = "accent-color";

#[derive(DeserializeDict, SerializeDict, Clone, Copy, PartialEq, Type)]
#[zvariant(signature = "dict")]
struct Color {
    color: [f64; 3],
}

impl Into<OwnedValue> for Color {
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
pub struct SettingsBackend {
    config: config::SettingsConfig,
}

impl SettingsBackend {
    pub fn init() -> Self {
        Self {
            config: config::SettingsConfig::config_from_file(),
        }
    }
}
#[dbus_interface(name = "org.freedesktop.impl.portal.Settings")]
impl SettingsBackend {
    #[dbus_interface(property, name = "version")]
    fn version(&self) -> u32 {
        1
    }

    fn read(&self, namespace: String, key: String) -> fdo::Result<OwnedValue> {
        if namespace != APPEARANCE {
            return Err(zbus::fdo::Error::Failed("No such namespace".to_string()));
        }
        if key == COLOR_SCHEME {
            return Ok(OwnedValue::from(self.config.get_color_scheme()));
        }
        if key == ACCENT_COLOR {
            return Ok(Color {
                color: self.config.get_accent_color(),
            }
            .into());
        }
        Err(zbus::fdo::Error::Failed("No such namespace".to_string()))
    }

    #[dbus_interface(signal)]
    async fn setting_changed(
        ctxt: &SignalContext<'_>,
        namespace: String,
        key: String,
        value: u32,
    ) -> zbus::Result<()>;
    // add code here
}
