use std::rc::Rc;

use slint::VecModel;
slint::include_modules!();

use std::sync::mpsc;

thread_local! {
    static GLOBAL_SELECT_UI : AppWindow = AppWindow::new().expect("Should can be init");
}

#[derive(Debug)]
pub enum SlintSelection {
    GlobalScreen { showcursor: bool },
    Slurp,
    Canceled,
    Selection { showcursor: bool, index: i32 },
}

fn init_slots(ui: &AppWindow, sender: mpsc::Sender<SlintSelection>) {
    let global = SelectSlots::get(ui);
    let sender_slurp = sender.clone();
    global.on_useSlurp(move || {
        let _ = sender_slurp.send(SlintSelection::Slurp);
        let _ = slint::quit_event_loop();
    });
    let sender_global = sender.clone();
    global.on_useGlobal(move |showcursor| {
        let _ = sender_global.send(SlintSelection::GlobalScreen { showcursor });
        let _ = slint::quit_event_loop();
    });

    global.on_selectScreen(move |index, showcursor| {
        let _ = sender.send(SlintSelection::Selection { index, showcursor });
        let _ = slint::quit_event_loop();
    });
}

pub fn selectgui(screen: Vec<ScreenInfo>) -> SlintSelection {
    GLOBAL_SELECT_UI.with(|ui| {
        ui.set_infos(Rc::new(VecModel::from(screen)).into());
        let (sender, receiver) = mpsc::channel();
        init_slots(&ui, sender);
        ui.run().unwrap();
        if let Ok(message) = receiver.recv_timeout(std::time::Duration::from_nanos(300)) {
            message
        } else {
            SlintSelection::Canceled
        }
    })
}
