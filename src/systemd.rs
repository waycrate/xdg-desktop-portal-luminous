use serde::Deserialize;
use zbus::{Result, zvariant};

const APP_SCOPE_PREFIX: &str = "app-";
const SCOPE_SUFFIX: &str = ".scope";
const KNOWN_LAUNCHERS: &[&str] = &["flatpak"];

#[zbus::proxy(
    default_service = "org.freedesktop.systemd1",
    default_path = "/org/freedesktop/systemd1",
    interface = "org.freedesktop.systemd1.Manager"
)]
pub trait Systemd1 {
    fn list_units(&self) -> Result<Vec<Unit>>;
    fn subscribe(&self) -> Result<()>;

    #[zbus(signal)]
    fn unit_new(&self, id: &str, unit: zvariant::OwnedObjectPath) -> Result<()>;

    #[zbus(signal)]
    fn unit_removed(&self, id: &str, unit: zvariant::OwnedObjectPath) -> Result<()>;
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, zvariant::Type)]
#[zvariant(signature = "(ssssssouso)")]
pub struct Unit {
    pub name: String,
    pub description: String,
    pub load_state: String,
    pub active_state: String,
    pub sub_state: String,
    pub following: String,
    pub unit_object: zvariant::OwnedObjectPath,
    pub job_id: u32,
    pub job_type: String,
    pub job_object: zvariant::OwnedObjectPath,
}

impl Unit {
    pub fn app_id(&self) -> Option<&str> {
        parse_app_scope_name(&self.name)
    }
}

pub fn parse_app_scope_name(unit_name: &str) -> Option<&str> {
    let without_prefix = unit_name.strip_prefix(APP_SCOPE_PREFIX)?;
    let mut without_suffix = without_prefix.strip_suffix(SCOPE_SUFFIX)?;
    without_suffix = KNOWN_LAUNCHERS
        .iter()
        .find_map(|launcher| without_suffix.strip_prefix(&format!("{launcher}-")))
        .unwrap_or(without_suffix);

    let (app_id, suffix) = without_suffix.rsplit_once('-')?;
    if app_id.is_empty() || !is_generated_scope_suffix(suffix) {
        return None;
    }

    Some(app_id)
}

fn is_generated_scope_suffix(suffix: &str) -> bool {
    !suffix.is_empty()
        && (suffix.bytes().all(|b| b.is_ascii_digit())
            || suffix.len() >= 4
                && suffix
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()))
}
