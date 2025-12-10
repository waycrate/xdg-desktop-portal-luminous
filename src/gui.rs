use iced::futures::channel::mpsc::Sender;
use iced::widget::{button, column, row, text};
use iced::{Alignment, Element, Length, Task as Command};
use iced_layershell::daemon;
use iced_layershell::reexport::{
    Anchor, KeyboardInteractivity, NewLayerShellSettings, OutputOption,
};
use iced_layershell::settings::{LayerShellSettings, StartMode};
use iced_layershell::to_layer_message;

use libwayshot::output::OutputInfo;
use libwayshot::region::TopLevel;

pub fn gui(toplevel_capture_support: bool) -> Result<(), iced_layershell::Error> {
    unsafe { std::env::set_var("RUST_LOG", "xdg-desktop-protal-luminous=info") }
    tracing_subscriber::fmt().init();
    tracing::info!("luminous Start");
    daemon(
        move || AreaSelectorGUI::new(toplevel_capture_support),
        AreaSelectorGUI::namespace,
        AreaSelectorGUI::update,
        AreaSelectorGUI::view,
    )
    .layer_settings({
        LayerShellSettings {
            size: Some((400, 400)),
            exclusive_zone: 0,
            anchor: Anchor::Bottom | Anchor::Left | Anchor::Right | Anchor::Top,
            keyboard_interactivity: KeyboardInteractivity::OnDemand,
            start_mode: StartMode::Background,
            ..Default::default()
        }
    })
    .subscription(AreaSelectorGUI::subscription)
    .run()
}

#[derive(Debug, Default, Clone, Copy, PartialEq)]
enum ViewMode {
    #[default]
    Screens,
    Windows,
}

#[derive(Debug, Default)]
struct AreaSelectorGUI {
    mode: ViewMode,
    window_show: bool,
    toplevel_capture_support: bool,
    sender: Option<Sender<CopySelect>>,
    toplevels: Vec<TopLevel>,
    screens: Vec<OutputInfo>,
}

#[derive(Debug, Clone)]
pub enum CopySelect {
    Window(usize),
    Screen(usize),
}

#[to_layer_message(multi)]
#[derive(Debug, Clone)]
pub enum Message {
    ScreenShotOpen {
        toplevels: Vec<TopLevel>,
        screens: Vec<OutputInfo>,
    },
    Selected {
        id: iced::window::Id,
        select: CopySelect,
    },
    ShowScreens,
    ShowWindows,
    ReadyShoot(Sender<CopySelect>),
}

impl AreaSelectorGUI {
    fn new(toplevel_capture_support: bool) -> Self {
        Self {
            mode: ViewMode::Screens,
            window_show: false,
            toplevel_capture_support,
            sender: None,
            toplevels: Vec::new(),
            screens: Vec::new(),
        }
    }

    fn namespace() -> String {
        String::from("Area Selector")
    }

    fn update(&mut self, message: Message) -> Command<Message> {
        match message {
            Message::ShowScreens => {
                self.mode = ViewMode::Screens;
                Command::none()
            }
            Message::ShowWindows => {
                self.mode = ViewMode::Windows;
                Command::none()
            }
            Message::Selected { id, select } => {
                use iced_runtime::Action;
                use iced_runtime::window::Action as WindowAction;
                let _ = self.sender.as_mut().unwrap().try_send(select);
                self.window_show = false;
                iced_runtime::task::effect(Action::Window(WindowAction::Close(id)))
            }

            Message::ScreenShotOpen { toplevels, screens } => {
                if self.window_show {
                    return Command::none();
                }
                self.window_show = true;
                self.toplevels = toplevels;
                self.screens = screens;
                Command::done(Message::NewLayerShell {
                    settings: NewLayerShellSettings {
                        exclusive_zone: None,
                        anchor: Anchor::Right | Anchor::Top | Anchor::Left | Anchor::Bottom,
                        size: Some((400, 400)),
                        margin: Some((100, 100, 100, 100)),
                        keyboard_interactivity: KeyboardInteractivity::OnDemand,
                        output_option: OutputOption::None,
                        ..Default::default()
                    },
                    id: iced::window::Id::unique(),
                })
            }
            Message::ReadyShoot(sender) => {
                self.sender = Some(sender);
                Command::none()
            }
            _ => unreachable!(),
        }
    }

    fn view(&self, id: iced::window::Id) -> Element<'_, Message> {
        let selector = row![
            button(text("Screen").center().width(Length::Fill))
                .on_press(Message::ShowScreens)
                .width(Length::Fill)
                .style(if self.mode == ViewMode::Screens {
                    button::primary
                } else {
                    button::secondary
                }),
            button(text("Window").center().width(Length::Fill))
                .on_press_maybe(if self.toplevel_capture_support {
                    Some(Message::ShowWindows)
                } else {
                    None
                })
                .width(Length::Fill)
                .style(if self.mode == ViewMode::Windows {
                    button::primary
                } else {
                    button::secondary
                }),
        ]
        .align_y(Alignment::Center)
        .spacing(10)
        .padding(20)
        .width(Length::Fill);

        let content: Element<'_, Message> = match self.mode {
            ViewMode::Screens => column(self.screens.iter().enumerate().map(|(index, info)| {
                button(text(&info.name).center().width(Length::Fill))
                    .on_press(Message::Selected {
                        id,
                        select: CopySelect::Screen(index),
                    })
                    .style(button::subtle)
                    .into()
            }))
            .spacing(10)
            .into(),
            ViewMode::Windows => column(self.toplevels.iter().enumerate().map(|(index, info)| {
                button(text(info.id_and_title()).center().width(Length::Fill))
                    .on_press(Message::Selected {
                        id,
                        select: CopySelect::Window(index),
                    })
                    .style(button::subtle)
                    .into()
            }))
            .spacing(10)
            .into(),
        };

        column![selector, content]
            .padding(20)
            .spacing(10)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn subscription(&self) -> iced::Subscription<Message> {
        iced::Subscription::run(|| {
            iced::stream::channel(100, |mut output: Sender<Message>| async move {
                use iced::futures::channel::mpsc::channel;
                use iced::futures::sink::SinkExt;
                let (sender, receiver) = channel(100);
                let _ = output.send(Message::ReadyShoot(sender)).await;

                let _ = crate::backend::backend(output, receiver).await;
            })
        })
    }
}
