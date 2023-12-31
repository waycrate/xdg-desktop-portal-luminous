use serde::Deserialize;
use std::io::Read;
const DEFAULT_COLOR_NAME: &str = "default";
const DARK_COLOR_NAME: &str = "dark";
const LIGHT_COLOR_NAME: &str = "light";

const DEFAULT_ACCENT_COLLOR: &str = "#ffffff";

#[derive(Deserialize, PartialEq, Eq, Debug)]
pub struct SettingsConfig {
    pub color_scheme: String,
    pub accent_color: String,
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
}

impl Default for SettingsConfig {
    fn default() -> Self {
        SettingsConfig {
            color_scheme: DEFAULT_COLOR_NAME.to_string(),
            accent_color: DEFAULT_ACCENT_COLLOR.to_string(),
        }
    }
}

impl SettingsConfig {
    pub fn config_from_file() -> Self {
        let Ok(home) = std::env::var("HOME") else {
            return Self::default();
        };
        let config_path = std::path::Path::new(home.as_str())
            .join(".config")
            .join("xdg-desktop-portal-luminous")
            .join("config.toml");
        let Ok(mut file) = std::fs::OpenOptions::new().read(true).open(config_path) else {
            return Self::default();
        };
        let mut buf = String::new();
        if file.read_to_string(&mut buf).is_err() {
            return Self::default();
        };
        toml::from_str(&buf).unwrap_or(Self::default())
    }
}
