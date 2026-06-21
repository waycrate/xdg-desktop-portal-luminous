use serde::Deserialize;
use std::io::Read;
use std::path::PathBuf;
use std::sync::LazyLock;

const DEFAULT_COLOR_NAME: &str = "default";
const DARK_COLOR_NAME: &str = "dark";
const LIGHT_COLOR_NAME: &str = "light";

const DEFAULT_ACCENT_COLLOR: &str = "#ffffff";

const DEFAULT_CONTRAST: &str = "default";
const HIGHER_CONTRAST: &str = "higher";

const DEFAULT_REDUCED_MOTION: &str = "default";
const REDUCED_REDUCED_MOTION: &str = "reduced";
const DEFAULT_BACKGROUND_PERMISSION: &str = "ask";

#[derive(Deserialize, PartialEq, Eq, Debug)]
pub struct SettingsConfig {
    pub color_scheme: String,
    pub accent_color: String,
    pub contrast: String,
    pub reduced_motion: String,
    pub screenshot_permission_check: bool,
    pub background_permission_default: String,
}

#[derive(Deserialize, PartialEq, Eq, Debug)]
struct SettingsConfigRead {
    pub color_scheme: Option<String>,
    pub accent_color: Option<String>,
    pub contrast: Option<String>,
    pub reduced_motion: Option<String>,
    pub screenshot_permission_check: Option<bool>,
    pub background_permission_default: Option<String>,
}

impl From<SettingsConfigRead> for SettingsConfig {
    fn from(value: SettingsConfigRead) -> Self {
        Self {
            color_scheme: value.color_scheme.unwrap_or(DEFAULT_COLOR_NAME.to_string()),
            accent_color: value
                .accent_color
                .unwrap_or(DEFAULT_ACCENT_COLLOR.to_string()),
            contrast: value.contrast.unwrap_or(DEFAULT_CONTRAST.to_string()),
            reduced_motion: value
                .reduced_motion
                .unwrap_or(DEFAULT_REDUCED_MOTION.to_string()),
            screenshot_permission_check: value.screenshot_permission_check.unwrap_or(true),
            background_permission_default: match value
                .background_permission_default
                .unwrap_or(DEFAULT_BACKGROUND_PERMISSION.to_string())
                .as_str()
            {
                "allow" => "allow".to_string(),
                "deny" => "deny".to_string(),
                _ => DEFAULT_BACKGROUND_PERMISSION.to_string(),
            },
        }
    }
}

impl SettingsConfig {
    pub fn get_color_scheme(&self) -> u32 {
        match self.color_scheme.as_str() {
            DEFAULT_COLOR_NAME => super::DEFAULT_COLOR,
            DARK_COLOR_NAME => super::DARK_COLOR,
            LIGHT_COLOR_NAME => super::LIGHT_COLOR,
            _ => unreachable!(),
        }
    }
    pub fn get_accent_color(&self) -> [f64; 3] {
        let color = csscolorparser::parse(&self.accent_color)
            .map(|color| color.to_rgba8())
            .unwrap_or(
                csscolorparser::parse(DEFAULT_ACCENT_COLLOR)
                    .unwrap()
                    .to_rgba8(),
            );
        [
            color[0] as f64 / 256.0,
            color[1] as f64 / 256.0,
            color[2] as f64 / 256.0,
        ]
    }
    pub fn get_contrast(&self) -> u32 {
        match self.contrast.as_str() {
            DEFAULT_CONTRAST => super::DEFAULT_CONTRAST,
            HIGHER_CONTRAST => super::HIGHER_CONTRAST,
            _ => super::DEFAULT_CONTRAST,
        }
    }
    pub fn get_reduced_motion(&self) -> u32 {
        match self.reduced_motion.as_str() {
            DEFAULT_REDUCED_MOTION => super::DEFAULT_REDUCED_MOTION,
            REDUCED_REDUCED_MOTION => super::REDUCED_REDUCED_MOTION,
            _ => super::DEFAULT_REDUCED_MOTION,
        }
    }
}

impl Default for SettingsConfig {
    fn default() -> Self {
        SettingsConfig {
            color_scheme: DEFAULT_COLOR_NAME.to_string(),
            accent_color: DEFAULT_ACCENT_COLLOR.to_string(),
            contrast: DEFAULT_CONTRAST.to_string(),
            reduced_motion: DEFAULT_REDUCED_MOTION.to_string(),
            screenshot_permission_check: true,
            background_permission_default: DEFAULT_BACKGROUND_PERMISSION.to_string(),
        }
    }
}

const PORTAL_CONFIG_FILE_NAME: &str = "config.toml";
const PORTAL_CONFIG_DIR_NAME: &str = "xdg-desktop-portal-luminous";

pub static XDG_CONFIG_HOME: LazyLock<Option<PathBuf>> = LazyLock::new(|| {
    if let Ok(xdg_config_home) = std::env::var("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(&xdg_config_home));
    }
    let home = std::env::var("HOME").ok()?;
    Some(PathBuf::from(&home).join(".config"))
});

pub static XDG_CONFIG_HOME_FILE: LazyLock<Option<PathBuf>> = LazyLock::new(|| {
    Some(
        XDG_CONFIG_HOME
            .clone()?
            .join(PORTAL_CONFIG_DIR_NAME)
            .join(PORTAL_CONFIG_FILE_NAME),
    )
});

pub static PORTAL_ETC_CONFIG_FILE: LazyLock<PathBuf> = LazyLock::new(|| {
    PathBuf::from("/etc")
        .join("xdg")
        .join(PORTAL_CONFIG_DIR_NAME)
        .join(PORTAL_CONFIG_FILE_NAME)
});

pub static PORTAL_CONFIG_FILE: LazyLock<PathBuf> = LazyLock::new(|| {
    if let Some(config_file) = XDG_CONFIG_HOME_FILE.clone() {
        return config_file;
    }
    PORTAL_ETC_CONFIG_FILE.clone()
});

impl SettingsConfig {
    pub fn config_from_file() -> Self {
        let Ok(mut file) = std::fs::OpenOptions::new()
            .read(true)
            .open(&*PORTAL_CONFIG_FILE)
        else {
            return Self::default();
        };
        let mut buf = String::new();
        if file.read_to_string(&mut buf).is_err() {
            return Self::default();
        };
        let Ok(file_config) = toml::from_str::<SettingsConfigRead>(&buf) else {
            return Self::default();
        };
        file_config.into()
    }
}
