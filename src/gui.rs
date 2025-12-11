use iced::futures::channel::mpsc::Sender;
use iced::widget::{Space, button, checkbox, column, container, row, scrollable, text};
use iced::{Alignment, Element, Length, Task};
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
    .layer_settings(LayerShellSettings {
        size: Some((400, 400)),
        exclusive_zone: 0,
        anchor: Anchor::Bottom | Anchor::Left | Anchor::Right | Anchor::Top,
        keyboard_interactivity: KeyboardInteractivity::OnDemand,
        start_mode: StartMode::Background,
        ..Default::default()
    })
    .subscription(AreaSelectorGUI::subscription)
    .run()
}

#[allow(unused)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum GuiMode {
    ScreenCast,
    #[default]
    ScreenShot,
}

#[derive(Debug, Default, Clone, Copy, PartialEq)]
enum ViewMode {
    #[default]
    Screens,
    Windows,
    Others,
}

#[derive(Debug, Default)]
struct AreaSelectorGUI {
    gui_mode: GuiMode,
    mode: ViewMode,
    window_show: bool,
    toplevel_capture_support: bool,
    sender: Option<Sender<CopySelect>>,
    toplevels: Vec<TopLevel>,
    screens: Vec<OutputInfo>,
    use_curor: bool,
}

#[derive(Debug, Clone)]
pub enum CopySelect {
    Window(usize),
    Screen(usize),
    All,
    Slurp,
    Cancel,
}

#[to_layer_message(multi)]
#[derive(Debug, Clone)]
pub enum Message {
    ImageCopyOpen {
        copytype: GuiMode,
        toplevels: Vec<TopLevel>,
        screens: Vec<OutputInfo>,
    },
    #[allow(unused)]
    Selected {
        id: iced::window::Id,
        use_curor: bool,
        select: CopySelect,
    },
    ShowScreens,
    ShowWindows,
    ShowOthers,
    ReadyShoot(Sender<CopySelect>),
    ToggleCursor(bool),
}

impl AreaSelectorGUI {
    fn new(toplevel_capture_support: bool) -> Self {
        Self {
            gui_mode: GuiMode::ScreenShot,
            mode: ViewMode::Screens,
            window_show: false,
            toplevel_capture_support,
            sender: None,
            toplevels: Vec::new(),
            screens: Vec::new(),
            use_curor: false,
        }
    }

    fn namespace() -> String {
        String::from("Area Selector")
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::ShowScreens => {
                self.mode = ViewMode::Screens;
                Task::none()
            }
            Message::ShowWindows => {
                self.mode = ViewMode::Windows;
                Task::none()
            }
            Message::ShowOthers => {
                self.mode = ViewMode::Others;
                Task::none()
            }
            Message::Selected { id, select, .. } => {
                use iced_runtime::Action;
                use iced_runtime::window::Action as WindowAction;
                let _ = self.sender.as_mut().unwrap().try_send(select);
                self.window_show = false;
                iced_runtime::task::effect(Action::Window(WindowAction::Close(id)))
            }

            Message::ImageCopyOpen {
                copytype,
                toplevels,
                screens,
            } => {
                if self.window_show {
                    return Task::none();
                }
                self.gui_mode = copytype;
                self.window_show = true;
                self.toplevels = toplevels;
                self.screens = screens;
                Task::done(Message::NewLayerShell {
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
                Task::none()
            }
            Message::ToggleCursor(cursor) => {
                self.use_curor = cursor;
                Task::none()
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
            button(text("Others").center().width(Length::Fill))
                .on_press_maybe(if self.gui_mode == GuiMode::ScreenShot {
                    Some(Message::ShowOthers)
                } else {
                    None
                })
                .width(Length::Fill)
                .style(if self.mode == ViewMode::Others {
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
            ViewMode::Screens => scrollable(
                column(self.screens.iter().enumerate().map(|(index, info)| {
                    button(text(&info.name).center().width(Length::Fill))
                        .on_press(Message::Selected {
                            id,
                            select: CopySelect::Screen(index),
                            use_curor: self.use_curor,
                        })
                        .style(button::subtle)
                        .into()
                }))
                .spacing(10),
            )
            .height(Length::Fill)
            .into(),
            ViewMode::Windows => scrollable(
                column(self.toplevels.iter().enumerate().map(|(index, info)| {
                    button(text(info.id_and_title()).center().width(Length::Fill))
                        .on_press(Message::Selected {
                            id,
                            select: CopySelect::Window(index),
                            use_curor: self.use_curor,
                        })
                        .style(button::subtle)
                        .into()
                }))
                .spacing(10),
            )
            .height(Length::Fill)
            .into(),
            ViewMode::Others => column![
                button(
                    container(text("Area Select"))
                        .center_y(Length::Fill)
                        .center_x(Length::Fill)
                )
                .height(Length::Fill)
                .width(Length::Fill)
                .on_press(Message::Selected {
                    id,
                    use_curor: self.use_curor,
                    select: CopySelect::Slurp
                })
                .style(button::subtle),
                button(
                    container(text("All"))
                        .center_y(Length::Fill)
                        .center_x(Length::Fill)
                )
                .height(Length::Fill)
                .width(Length::Fill)
                .on_press(Message::Selected {
                    id,
                    use_curor: self.use_curor,
                    select: CopySelect::All
                })
                .style(button::subtle),
            ]
            .spacing(10)
            .height(Length::Fill)
            .into(),
        };

        let bottom_button_list: Element<'_, Message> = container(row![
            Space::new().width(Length::Fill),
            container(
                checkbox(self.use_curor)
                    .label("use_curor")
                    .on_toggle(Message::ToggleCursor)
            )
            .center_y(Length::Fill),
            Space::new().width(Length::Fixed(2.)),
            button(text("Cancel")).on_press(Message::Selected {
                id,
                use_curor: self.use_curor,
                select: CopySelect::Cancel
            })
        ])
        .center_y(Length::Fixed(30.0))
        .into();

        column![selector, content, bottom_button_list]
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
