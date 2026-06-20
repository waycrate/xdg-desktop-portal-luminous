use std::sync::{Arc, Mutex};

use futures::{SinkExt, channel::mpsc::Sender};
use tokio::sync::oneshot;
use zbus::{interface, zvariant::OwnedObjectPath};

use crate::dialog::Message;

pub struct RequestCloseAction {
    pub cancel_sender: Arc<Mutex<Option<oneshot::Sender<()>>>>,
    pub ui_sender: Sender<Message>,
    pub close_message: Option<Message>,
}

pub struct RequestInterface {
    pub handle_path: OwnedObjectPath,
    pub close_action: Option<RequestCloseAction>,
}

#[interface(name = "org.freedesktop.impl.portal.Request")]
impl RequestInterface {
    async fn close(
        &self,
        #[zbus(object_server)] server: &zbus::ObjectServer,
    ) -> zbus::fdo::Result<()> {
        if let Some(action) = &self.close_action {
            let cancel_sender = action
                .cancel_sender
                .lock()
                .ok()
                .and_then(|mut sender| sender.take());

            if let Some(cancel_sender) = cancel_sender {
                let _ = cancel_sender.send(());
            }

            if let Some(close_message) = &action.close_message {
                let mut ui_sender = action.ui_sender.clone();
                let _ = ui_sender.send(close_message.clone()).await;
            }
        }

        server
            .remove::<Self, &OwnedObjectPath>(&self.handle_path)
            .await?;
        Ok(())
    }
}
