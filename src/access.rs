use std::collections::HashMap;

use zbus::{
    dbus_interface, fdo,
    zvariant::{DeserializeDict, ObjectPath, OwnedValue, SerializeDict, Type},
};

use serde::{Deserialize, Serialize};

use crate::PortalResponse;
#[derive(SerializeDict, DeserializeDict, Type, Debug, Default)]
/// Specified options for a [`Screencast::select_sources`] request.
#[zvariant(signature = "dict")]
pub struct AccessOption {
    /// A string that will be used as the last element of the handle.
    /// What types of content to record.
    pub modal: Option<bool>,
    /// Whether to allow selecting multiple sources.
    pub deny_label: Option<String>,
    /// Determines how the cursor will be drawn in the screen cast stream.
    pub grant_label: Option<String>,
    pub icon: Option<String>,
    pub choices: Option<Vec<Choice>>,
}

#[derive(Clone, Serialize, Deserialize, Type, Debug)]
/// Presents the user with a choice to select from or as a checkbox.
pub struct Choice(String, String, Vec<(String, String)>, String);

#[derive(Debug)]
pub struct AccessBackend;

#[dbus_interface(name = "org.freedesktop.impl.portal.Access")]
impl AccessBackend {
    #[allow(clippy::too_many_arguments)]
    async fn access_dialog(
        &self,
        _request_handle: ObjectPath<'_>,
        _app_id: String,
        _parrent_window: String,
        title: String,
        sub_title: String,
        _body: String,
        _options: AccessOption,
    ) -> fdo::Result<PortalResponse<HashMap<String, OwnedValue>>> {
        if accessdialog::confirmgui(title, sub_title) {
            Ok(PortalResponse::Success(HashMap::new()))
        } else {
            Ok(PortalResponse::Cancelled)
        }
    }
    // add code here
}
