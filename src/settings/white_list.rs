use super::config::XDG_CONFIG_HOME_DIR;
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::sync::LazyLock;

use std::path::PathBuf;
use tokio::sync::Mutex;
const WHITE_LIST_FILE_NAME: &str = "whitelist.json";

static WHIT_LIST_FILE: LazyLock<Option<PathBuf>> =
    LazyLock::new(|| Some(XDG_CONFIG_HOME_DIR.clone()?.join(WHITE_LIST_FILE_NAME)));

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct WhiteList {
    #[serde(default)]
    screen_shot: Vec<String>,
    #[serde(default)]
    remote: Vec<String>,
}

#[derive(Debug)]
pub struct WhiteListMaintainer {
    list: Mutex<WhiteList>,
}

pub static WHITE_LIST_MAINTAINER: LazyLock<WhiteListMaintainer> =
    LazyLock::new(WhiteListMaintainer::new);

impl WhiteListMaintainer {
    fn new() -> Self {
        Self {
            list: Mutex::new(WhiteList::config_from_file()),
        }
    }
    pub async fn check_shot(&self, app: &str) -> bool {
        let list = self.list.lock().await;
        list.screen_shot.contains(&app.to_string())
    }
    pub async fn check_remote(&self, app: &str) -> bool {
        let list = self.list.lock().await;
        list.remote.contains(&app.to_string())
    }
    pub async fn add_screenshot_whitelist(&self, app: &str) {
        let mut list = self.list.lock().await;
        list.screen_shot.push(app.to_string());
        list.save_to_file();
    }
    pub async fn add_remote_whitelist(&self, app: &str) {
        let mut list = self.list.lock().await;
        list.remote.push(app.to_string());
        list.save_to_file();
    }
}
impl WhiteList {
    fn config_from_file() -> Self {
        let Some(whitelist_file) = WHIT_LIST_FILE.clone() else {
            return Self::default();
        };
        let Ok(mut file) = std::fs::OpenOptions::new().read(true).open(whitelist_file) else {
            return Self::default();
        };
        let mut buf = String::new();
        if file.read_to_string(&mut buf).is_err() {
            return Self::default();
        };
        serde_json::from_str(&buf).unwrap_or_default()
    }
    fn save_to_file(&self) -> Option<()> {
        let xdg_home_dir = XDG_CONFIG_HOME_DIR.clone()?;
        std::fs::create_dir_all(&xdg_home_dir).ok()?;
        let whitelist_file = xdg_home_dir.join(WHITE_LIST_FILE_NAME);
        let data = serde_json::to_string_pretty(&self).ok()?;
        std::fs::write(whitelist_file, data).ok()
    }
}
