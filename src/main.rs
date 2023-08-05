use libwayshot::WayshotConnection;
use std::collections::HashMap;
use std::future::pending;
use zbus::zvariant::{DeserializeDict, SerializeDict, Type, Value};
use zbus::{dbus_interface, fdo, zvariant::ObjectPath, ConnectionBuilder};

#[derive(DeserializeDict, SerializeDict, Type)]
#[zvariant(signature = "dict")]
pub struct Screenshot {
    uri: url::Url,
}

#[derive(DeserializeDict, SerializeDict, Clone, Copy, PartialEq, Type)]
#[zvariant(signature = "dict")]
pub struct Color {
    color: [f64; 3],
}

#[derive(DeserializeDict, SerializeDict, Type, Debug)]
#[zvariant(signature = "dict")]
pub struct ScreenshotOption {
    interactive: bool,
    modal: bool,
}

struct ShanaShot {
    wayshot_connection: WayshotConnection,
}

#[dbus_interface(name = "org.freedesktop.impl.portal.Screenshot")]
impl ShanaShot {
    fn screenshot(
        &mut self,
        _handle: ObjectPath<'_>,
        _app_id: String,
        _parent_window: String,
        options: ScreenshotOption,
    ) -> fdo::Result<(u32, Screenshot)> {
        let image_buffer = self
            .wayshot_connection
            .screenshot_all(options.interactive)
            .map_err(|e| zbus::Error::Failure(format!("Wayland screencopy failed, {e}")))?;

        image_buffer.save("/tmp/wayshot.png").map_err(|e| {
            zbus::Error::Failure(format!("Cannot save to /tmp/wayshot.png, e: {e}"))
        })?;

        Ok((
            0,
            Screenshot {
                uri: url::Url::from_file_path("/tmp/wayshot.png").unwrap(),
            },
        ))
    }

    fn pick_color(
        &mut self,
        _handle: ObjectPath<'_>,
        _app_id: String,
        _parent_window: String,
        _options: HashMap<String, Value<'_>>,
    ) -> fdo::Result<(u32, Color)> {
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

        let image = self
            .wayshot_connection
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
        Ok((
            0,
            Color {
                color: [
                    pixel.0[0] as f64 / 256.0,
                    pixel.0[1] as f64 / 256.0,
                    pixel.0[2] as f64 / 256.0,
                ],
            },
        ))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    std::env::set_var("RUST_LOG", "xdg-desktop-protal-wlrrust=info");
    tracing_subscriber::fmt().init();
    tracing::info!("wlrrust Start");
    let _conn = ConnectionBuilder::session()?
        .name("org.freedesktop.impl.portal.desktop.wlrrust")?
        .serve_at(
            "/org/freedesktop/portal/desktop",
            ShanaShot {
                wayshot_connection: WayshotConnection::new().unwrap(),
            },
        )?
        .build()
        .await?;

    pending::<()>().await;
    Ok(())
}
