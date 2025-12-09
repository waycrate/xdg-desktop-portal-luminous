use std::collections::HashMap;

use zbus::{
    fdo, interface,
    zvariant::{ObjectPath, OwnedValue, Type, as_value::optional},
};

use serde::{Deserialize, Serialize};

use crate::PortalResponse;
#[derive(Type, Debug, Default, Deserialize, Serialize)]
/// Specified options for a [`Screencast::select_sources`] request.
#[zvariant(signature = "dict")]
pub struct AccessOption {
    /// A string that will be used as the last element of the handle.
    /// What types of content to record.
    #[serde(with = "optional", skip_serializing_if = "Option::is_none", default)]
    pub modal: Option<bool>,
    /// Whether to allow selecting multiple sources.
    #[serde(with = "optional", skip_serializing_if = "Option::is_none", default)]
    pub deny_label: Option<String>,
    /// Determines how the cursor will be drawn in the screen cast stream.
    #[serde(with = "optional", skip_serializing_if = "Option::is_none", default)]
    pub grant_label: Option<String>,
    #[serde(with = "optional", skip_serializing_if = "Option::is_none", default)]
    pub icon: Option<String>,
    #[serde(with = "optional", skip_serializing_if = "Option::is_none", default)]
    pub choices: Option<Vec<Choice>>,
}

#[derive(Clone, Serialize, Deserialize, Type, Debug)]
/// Presents the user with a choice to select from or as a checkbox.
pub struct Choice(String, String, Vec<(String, String)>, String);

#[derive(Debug)]
pub struct AccessBackend;

#[interface(name = "org.freedesktop.impl.portal.Access")]
impl AccessBackend {
    #[allow(clippy::too_many_arguments)]
    async fn access_dialog(
        &self,
        _request_handle: ObjectPath<'_>,
        _app_id: String,
        _parent_window: String,
        _title: String,
        _sub_title: String,
        _body: String,
        _options: AccessOption,
    ) -> fdo::Result<PortalResponse<HashMap<String, OwnedValue>>> {
        Ok(PortalResponse::Success(HashMap::new()))
    }
    // add code here
}
