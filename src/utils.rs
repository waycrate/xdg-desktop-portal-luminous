use std::path::PathBuf;

use once_cell::sync::Lazy;

use uzers::get_current_uid;

pub static USER_RUNNING_DIR: Lazy<PathBuf> = Lazy::new(|| {
    let uid = get_current_uid();
    PathBuf::from("/run/user").join(uid.to_string())
});
