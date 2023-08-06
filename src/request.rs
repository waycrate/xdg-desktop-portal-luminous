use zbus::{dbus_interface, zvariant::OwnedObjectPath};

pub struct RequestInterface {
    pub handle_path: OwnedObjectPath,
}

#[dbus_interface(name = "org.freedesktop.impl.portal.Request")]
impl RequestInterface {
    async fn close(
        &self,
        #[zbus(object_server)] server: &zbus::ObjectServer,
    ) -> zbus::fdo::Result<()> {
        server
            .remove::<Self, &OwnedObjectPath>(&self.handle_path)
            .await?;
        Ok(())
    }
}
