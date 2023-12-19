use libwayshot::WayshotConnection;
use screenshotdialog::ScreenInfo;
use screenshotdialog::SlintSelection;
use std::collections::HashMap;
use zbus::zvariant::{DeserializeDict, SerializeDict, Type, Value};
use zbus::{dbus_interface, fdo, zvariant::ObjectPath};

use crate::utils::USER_RUNNING_DIR;
use crate::PortalResponse;

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

#[dbus_interface(name = "org.freedesktop.impl.portal.Screenshot")]
impl ScreenShotBackend {
    #[dbus_interface(property, name = "version")]
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
            let wayinfos = WayshotConnection::new()
                .map_err(|_| {
                    zbus::Error::Failure("Cannot create a new wayshot_connection".to_string())
                })?
                .get_all_outputs()
                .clone();
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
                    let slurp = std::process::Command::new("slurp")
                        .arg("-d")
                        .output()
                        .map_err(|_| zbus::Error::Failure("Cannot find slurp".to_string()))?
                        .stdout;

                    let output = String::from_utf8_lossy(&slurp).to_string();
                    let output = output.trim();
                    let output: Vec<&str> = output.split(' ').collect();
                    if output.len() < 2 {
                        return Err(zbus::Error::Failure("Illegal slurp input".to_string()).into());
                    }
                    let pos: Vec<&str> = output[0].split(',').collect();
                    if pos.len() < 2 {
                        return Err(
                            zbus::Error::Failure("Illegal slurp pos input".to_string()).into()
                        );
                    }
                    let region: Vec<&str> = output[1].split('x').collect();
                    if region.len() < 2 {
                        return Err(
                            zbus::Error::Failure("Illegal region pos input".to_string()).into()
                        );
                    }
                    wayshot_connection
                        .screenshot(
                            libwayshot::CaptureRegion {
                                x_coordinate: pos[0].parse().map_err(|_| {
                                    zbus::Error::Failure("X is not correct".to_string())
                                })?,
                                y_coordinate: pos[1].parse().map_err(|_| {
                                    zbus::Error::Failure("Y is not correct".to_string())
                                })?,
                                width: region[0].parse().map_err(|_| {
                                    zbus::Error::Failure("Width is not legel".to_string())
                                })?,
                                height: region[1].parse().map_err(|_| {
                                    zbus::Error::Failure("Height is not legel".to_string())
                                })?,
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
        let slurp = std::process::Command::new("slurp")
            .arg("-p")
            .output()
            .map_err(|_| zbus::Error::Failure("Cannot find slurp".to_string()))?
            .stdout;
        let output = String::from_utf8_lossy(&slurp);
        let output = output
            .split(' ')
            .next()
            .ok_or(zbus::Error::Failure("Not get slurp area".to_string()))?;
        let point: Vec<&str> = output.split(',').collect();

        let image = wayshot_connection
            .screenshot(
                libwayshot::CaptureRegion {
                    x_coordinate: point[0]
                        .parse()
                        .map_err(|_| zbus::Error::Failure("X is not correct".to_string()))?,
                    y_coordinate: point[1]
                        .parse()
                        .map_err(|_| zbus::Error::Failure("Y is not correct".to_string()))?,
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
