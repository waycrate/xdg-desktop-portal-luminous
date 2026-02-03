use libwayshot::{
    WayshotConnection,
    region::{LogicalRegion, Region, Size},
};
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

use crate::PortalResponse;
use crate::dialog::{CopySelect, Message, TopLevelInfo, WlOutputInfo};
use crate::settings::SETTING_CONFIG;
use crate::utils::USER_RUNNING_DIR;
use futures::{
    SinkExt, StreamExt,
    channel::mpsc::{Receiver, Sender},
};

use libwaysip::WaySip;

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
    #[serde(default)]
    permission_store_checked: bool,
}

#[derive(Debug)]
pub struct ScreenShotBackend {
    pub sender: Sender<Message>,
    pub receiver: Receiver<CopySelect>,
}

pub fn waysip_to_region(
    size: libwaysip::Size,
    position: libwaysip::Position,
) -> libwayshot::Result<LogicalRegion> {
    let size: Size = Size {
        width: size.width.try_into().map_err(|_| {
            libwayshot::Error::FreezeCallbackError("width cannot be negative".to_string())
        })?,
        height: size.height.try_into().map_err(|_| {
            libwayshot::Error::FreezeCallbackError("height cannot be negative".to_string())
        })?,
    };
    let position: libwayshot::region::Position = libwayshot::region::Position {
        x: position.x,
        y: position.y,
    };

    Ok(LogicalRegion {
        inner: Region { position, size },
    })
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
        if SETTING_CONFIG.lock().await.screenshot_permission_check
            && !options.permission_store_checked
        {
            self.sender
                .send(Message::PermissionDialog(format!(
                    "Allow '{}' to take a screenshot?",
                    app_id
                )))
                .await
                .map_err(|e| zbus::Error::Failure(e.to_string()))?;

            if self.receiver.next().await != Some(CopySelect::Permission(true)) {
                return Ok(PortalResponse::Cancelled);
            }
            // reserve time to let dialog disappear
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
        let wayshot_connection = WayshotConnection::new()
            .map_err(|_| zbus::Error::Failure("Cannot update outputInfos".to_string()))?;
        tracing::info!("Start shot: path :{}, appid: {}", handle.as_str(), app_id);
        let image_buffer = if options.interactive {
            let top_levels = wayshot_connection.get_all_toplevels();
            let screens = wayshot_connection.get_all_outputs();
            let top_levels_iced = top_levels
                .iter()
                .map(|level| TopLevelInfo {
                    top_level: level.clone(),
                    image: None,
                })
                .collect();
            let screens_iced = screens
                .iter()
                .map(|output| WlOutputInfo {
                    output: output.clone(),
                    image: None,
                })
                .collect();
            let _ = self
                .sender
                .send(Message::ImageCopyOpen {
                    top_levels: top_levels_iced,
                    screens: screens_iced,
                })
                .await;
            let Some(select) = self.receiver.next().await else {
                return Ok(PortalResponse::Cancelled);
            };
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            match select {
                CopySelect::Screen { index, show_cursor } => wayshot_connection
                    .screenshot_single_output(&screens[index], show_cursor)
                    .map_err(|e| zbus::Error::Failure(format!("Wayland screencopy failed, {e}")))?,
                CopySelect::Window { index, show_cursor } => wayshot_connection
                    .screenshot_toplevel(&top_levels[index], show_cursor)
                    .map_err(|e| zbus::Error::Failure(format!("Wayland screencopy failed, {e}")))?,
                CopySelect::All => wayshot_connection
                    .screenshot_all(false)
                    .map_err(|e| zbus::Error::Failure(format!("Wayland screencopy failed, {e}")))?,
                CopySelect::Slurp => wayshot_connection
                    .screenshot_freeze(
                        |w_conn| {
                            let info = WaySip::new()
                                .with_connection(w_conn.conn.clone())
                                .with_selection_type(libwaysip::SelectionType::Area)
                                .get()
                                .map_err(|e| libwayshot::Error::FreezeCallbackError(e.to_string()))?
                                .ok_or(libwayshot::Error::FreezeCallbackError(
                                    "Failed to capture the area".to_string(),
                                ))?;
                            waysip_to_region(info.size(), info.left_top_point())
                        },
                        false,
                    )
                    .map_err(|e| zbus::Error::Failure(format!("Wayland screencopy failed, {e}")))?,
                CopySelect::Cancel => {
                    return Ok(PortalResponse::Cancelled);
                }
                CopySelect::Permission(_) => unreachable!(),
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

    async fn pick_color(
        &mut self,
        _handle: ObjectPath<'_>,
        _app_id: String,
        _parent_window: String,
        _options: HashMap<String, Value<'_>>,
    ) -> fdo::Result<PortalResponse<Color>> {
        let wayshot_connection = WayshotConnection::new()
            .map_err(|_| zbus::Error::Failure("Cannot update outputInfos".to_string()))?;

        let image = wayshot_connection
            .screenshot_freeze(
                |w_conn| {
                    let info = WaySip::new()
                        .with_connection(w_conn.conn.clone())
                        .with_selection_type(libwaysip::SelectionType::Point)
                        .get()
                        .map_err(|e| libwayshot::Error::FreezeCallbackError(e.to_string()))?
                        .ok_or(libwayshot::Error::FreezeCallbackError(
                            "Failed to capture the area".to_string(),
                        ))?;
                    waysip_to_region(
                        libwaysip::Size {
                            width: 1,
                            height: 1,
                        },
                        info.left_top_point(),
                    )
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
