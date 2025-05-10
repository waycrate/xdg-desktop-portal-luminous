slint::include_modules!();

use std::sync::mpsc;

thread_local! {
    static GLOBAL_SELECT_UI : AppWindow = AppWindow::new().expect("Should can be inited");
}

fn init_slots(ui: &AppWindow, sender: mpsc::Sender<bool>) {
    let global = ConfirmSlots::get(ui);
    let send_confirm = sender.clone();
    global.on_Reject(move || {
        let _ = sender.send(false);
        let _ = slint::quit_event_loop();
    });
    global.on_Confirm(move || {
        let _ = send_confirm.send(true);
        let _ = slint::quit_event_loop();
    });
}

pub fn confirmgui(title: String, information: String) -> bool {
    GLOBAL_SELECT_UI.with(|ui| {
        ui.set_init_title(title.into());
        ui.set_information(information.into());
        let (sender, receiver) = mpsc::channel();
        init_slots(&ui, sender);
        receiver
            .recv_timeout(std::time::Duration::from_nanos(300))
            .unwrap_or_default()
    })
}
