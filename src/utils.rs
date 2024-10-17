use std::path::PathBuf;

use std::sync::LazyLock;

pub static USER_RUNNING_DIR: LazyLock<PathBuf> = LazyLock::new(|| {
    let cache_dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or("/tmp".to_string());
    PathBuf::from(cache_dir)
});
