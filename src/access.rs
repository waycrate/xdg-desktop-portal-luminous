use std::collections::HashMap;

use zbus::{
    dbus_interface, fdo,
    zvariant::{OwnedValue, Value},
};

use crate::PortalResponse;

#[derive(Debug)]
pub struct AccessBackend;

#[dbus_interface(name = "org.freedesktop.impl.portal.Access")]
impl AccessBackend {
    async fn access_dialog(
        &self,
        _app_id: String,
        _parrent_window: String,
        _title: String,
        _sub_title: String,
        _body: String,
        _options: HashMap<String, Value<'_>>,
    ) -> fdo::Result<PortalResponse<HashMap<String, OwnedValue>>> {
        Ok(PortalResponse::Success(HashMap::new()))
    }
    // add code here
}
