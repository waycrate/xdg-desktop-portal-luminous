use libwayshot::WayshotConnection;
use libwaysip::Point;
use screenshotdialog::ScreenInfo;
use screenshotdialog::SlintSelection;
use std::collections::HashMap;
use zbus::zvariant::{DeserializeDict, SerializeDict, Type, Value};
use zbus::{fdo, interface, zvariant::ObjectPath};

use crate::PortalResponse;
use crate::utils::USER_RUNNING_DIR;

use libwaysip::SelectionType;

#[derive(DeserializeDict, SerializeDict, Type)]
#[zvariant(signature = "dict")]
struct Screenshot {
    uri: url::Url,
}

#[derive(DeserializeDict, SerializeDict, Clone, Copy, PartialEq, Type)]
#[zvariant(signature = "dict")]
struct Color {
    color: [f64; 3],
}

#[derive(DeserializeDict, SerializeDict, Type, Debug)]
#[zvariant(signature = "dict")]
pub struct ScreenshotOption {
    interactive: bool,
    modal: Option<bool>,
    permission_store_checked: Option<bool>,
}

#[derive(Debug)]
pub struct ScreenShotBackend;

#[interface(name = "org.freedesktop.impl.portal.Screenshot")]
impl ScreenShotBackend {
    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        1
    }
    fn screenshot(
        &mut self,
        handle: ObjectPath<'_>,
        app_id: String,
        _parent_window: String,
        options: ScreenshotOption,
    ) -> fdo::Result<PortalResponse<Screenshot>> {
        tracing::info!("Start shot: path :{}, appid: {}", handle.as_str(), app_id);
        let wayshot_connection = WayshotConnection::new()
            .map_err(|_| zbus::Error::Failure("Cannot update outputInfos".to_string()))?;

        let image_buffer = if options.interactive {
            let wayinfos = wayshot_connection.get_all_outputs();
            let screen_infos = wayinfos
                .iter()
                .map(|screen| ScreenInfo {
                    name: screen.name.clone().into(),
                    description: screen.description.clone().into(),
                })
                .collect();
            match screenshotdialog::selectgui(screen_infos) {
                SlintSelection::Canceled => return Ok(PortalResponse::Cancelled),
                SlintSelection::Slurp => {
                    let info = match libwaysip::get_area(None, SelectionType::Area) {
                        Ok(Some(info)) => info,
                        Ok(None) => {
                            return Err(zbus::Error::Failure("You cancel it".to_string()).into());
                        }
                        Err(e) => {
                            return Err(zbus::Error::Failure(format!("wayland error, {e}")).into());
                        }
                    };

                    let Point {
                        x: x_coordinate,
                        y: y_coordinate,
                    } = info.left_top_point();
                    let width = info.width();
                    let height = info.height();

                    wayshot_connection
                        .screenshot(
                            libwayshot::CaptureRegion {
                                x_coordinate,
                                y_coordinate,
                                width,
                                height,
                            },
                            false,
                        )
                        .map_err(|e| {
                            zbus::Error::Failure(format!("Wayland screencopy failed, {e}"))
                        })?
                }
                SlintSelection::GlobalScreen { showcursor } => wayshot_connection
                    .screenshot_all(showcursor)
                    .map_err(|e| zbus::Error::Failure(format!("Wayland screencopy failed, {e}")))?,
                SlintSelection::Selection { index, showcursor } => wayshot_connection
                    .screenshot_single_output(&wayinfos[index as usize], showcursor)
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
        let info = match libwaysip::get_area(None, SelectionType::Point) {
            Ok(Some(info)) => info,
            Ok(None) => return Err(zbus::Error::Failure("You cancel it".to_string()).into()),
            Err(e) => return Err(zbus::Error::Failure(format!("wayland error, {e}")).into()),
        };
        let Point {
            x: x_coordinate,
            y: y_coordinate,
        } = info.left_top_point();

        let image = wayshot_connection
            .screenshot(
                libwayshot::CaptureRegion {
                    x_coordinate,
                    y_coordinate,
                    width: 1,
                    height: 1,
                },
                false,
            )
            .map_err(|e| zbus::Error::Failure(format!("Wayland screencopy failed, {e}")))?;

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
