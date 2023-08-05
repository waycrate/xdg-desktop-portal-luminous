use std::rc::Rc;

use libwayshot::output::OutputInfo;
use slint::VecModel;
slint::include_modules!();

use std::sync::mpsc;

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

pub fn selectgui(screen: Vec<OutputInfo>) -> SlintSelection {
    let ui = AppWindow::new().unwrap();
    ui.set_infos(
        Rc::new(VecModel::from(
            screen
                .iter()
                .map(|screen| ScreenInfo {
                    name: screen.name.clone().into(),
                    description: screen.name.clone().into(),
                })
                .collect::<Vec<ScreenInfo>>(),
        ))
        .into(),
    );
    let (sender, receiver) = mpsc::channel();
    init_slots(&ui, sender);
    ui.run().unwrap();
    if let Ok(message) = receiver.recv_timeout(std::time::Duration::from_nanos(300)) {
        message
    } else {
        SlintSelection::Canceled
    }
}
