use std::path::PathBuf;

use once_cell::sync::Lazy;

pub static USER_RUNNING_DIR: Lazy<PathBuf> = Lazy::new(|| {
    let cache_dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or("/tmp".to_string());
    PathBuf::from(cache_dir)
});
