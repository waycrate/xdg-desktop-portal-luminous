use libwayshot::{
    WayshotConnection, region,
    region::{LogicalRegion, Region, Size},
};
use libwaysip::Position;
use serde::Deserialize;
use serde::Serialize;
use std::collections::HashMap;
use zbus::zvariant::{Type, Value};
use zbus::{
    fdo, interface,
    zvariant::{
        ObjectPath,
        as_value::{self, optional},
    },
};

use crate::gui::Message;
use crate::utils::USER_RUNNING_DIR;
use crate::{PortalResponse, gui::CopySelect};
use futures::{
    SinkExt, StreamExt,
    channel::mpsc::{Receiver, Sender},
};

use libwaysip::{SelectionType, WaySip};

#[derive(Type, Serialize, Deserialize)]
#[zvariant(signature = "dict")]
struct Screenshot {
    #[serde(with = "as_value")]
    uri: url::Url,
}

#[derive(Clone, Copy, PartialEq, Type, Serialize, Deserialize)]
#[zvariant(signature = "dict")]
struct Color {
    #[serde(with = "as_value")]
    color: [f64; 3],
}

#[derive(Type, Debug, Serialize, Deserialize)]
#[zvariant(signature = "dict")]
pub struct ScreenshotOption {
    #[serde(with = "as_value")]
    interactive: bool,
    #[serde(with = "optional", skip_serializing_if = "Option::is_none", default)]
    modal: Option<bool>,
    #[serde(with = "optional", skip_serializing_if = "Option::is_none", default)]
    permission_store_checked: Option<bool>,
}

#[derive(Debug)]
pub struct ScreenShotBackend {
    pub sender: Sender<Message>,
    pub receiver: Receiver<CopySelect>,
}

#[interface(name = "org.freedesktop.impl.portal.Screenshot")]
impl ScreenShotBackend {
    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        1
    }
    async fn screenshot(
        &mut self,
        handle: ObjectPath<'_>,
        app_id: String,
        _parent_window: String,
        options: ScreenshotOption,
    ) -> fdo::Result<PortalResponse<Screenshot>> {
        let wayshot_connection = WayshotConnection::new()
            .map_err(|_| zbus::Error::Failure("Cannot update outputInfos".to_string()))?;
        tracing::info!("Start shot: path :{}, appid: {}", handle.as_str(), app_id);
        let image_buffer = if options.interactive {
            let top_levels = wayshot_connection.get_all_toplevels();
            let screens = wayshot_connection.get_all_outputs();
            let _ = self
                .sender
                .send(Message::ScreenShotOpen {
                    toplevels: top_levels.to_vec(),
                    screens: screens.to_vec(),
                })
                .await;
            let Some(select) = self.receiver.next().await else {
                return Ok(PortalResponse::Cancelled);
            };
            match select {
                CopySelect::Screen(index) => wayshot_connection
                    .screenshot_single_output(&screens[index], false)
                    .map_err(|e| zbus::Error::Failure(format!("Wayland screencopy failed, {e}")))?,
                CopySelect::Window(index) => wayshot_connection
                    .screenshot_toplevel(&top_levels[index], false)
                    .map_err(|e| zbus::Error::Failure(format!("Wayland screencopy failed, {e}")))?,
            }
        } else {
            wayshot_connection
                .screenshot_all(false)
                .map_err(|e| zbus::Error::Failure(format!("Wayland screencopy failed, {e}")))?
        };

        let savepath = USER_RUNNING_DIR.join("wayshot.png");
        image_buffer.save(&savepath).map_err(|e| {
            zbus::Error::Failure(format!("Cannot save to {}, e: {e}", savepath.display()))
        })?;
        tracing::info!("Shot Finished");
        Ok(PortalResponse::Success(Screenshot {
            uri: url::Url::from_file_path(savepath).unwrap(),
        }))
    }

    fn pick_color(
        &mut self,
        _handle: ObjectPath<'_>,
        _app_id: String,
        _parent_window: String,
        _options: HashMap<String, Value<'_>>,
    ) -> fdo::Result<PortalResponse<Color>> {
        let wayshot_connection = WayshotConnection::new()
            .map_err(|_| zbus::Error::Failure("Cannot update outputInfos".to_string()))?;
        let info = match WaySip::new()
            .with_selection_type(SelectionType::Point)
            .get()
        {
            Ok(Some(info)) => info,
            Ok(None) => return Err(zbus::Error::Failure("You cancel it".to_string()).into()),
            Err(e) => return Err(zbus::Error::Failure(format!("wayland error, {e}")).into()),
        };
        let Position {
            x: x_coordinate,
            y: y_coordinate,
        } = info.left_top_point();

        let image = wayshot_connection
            .screenshot(
                LogicalRegion {
                    inner: Region {
                        position: region::Position {
                            x: x_coordinate,
                            y: y_coordinate,
                        },
                        size: Size {
                            width: 1,
                            height: 1,
                        },
                    },
                },
                false,
            )
            .map_err(|e| zbus::Error::Failure(format!("Wayland screencopy failed, {e}")))?
            .to_rgba8();

        let pixel = image.get_pixel(0, 0);
        Ok(PortalResponse::Success(Color {
            color: [
                pixel.0[0] as f64 / 256.0,
                pixel.0[1] as f64 / 256.0,
                pixel.0[2] as f64 / 256.0,
            ],
        }))
    }
}
