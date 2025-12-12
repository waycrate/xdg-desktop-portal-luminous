use iced::futures::channel::mpsc::Sender;
use iced::widget::{Row, Space, button, checkbox, column, container, image, row, scrollable, text};
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
    sender_cast: Option<Sender<CopySelect>>,
    toplevels: Vec<TopLevelInfo>,
    screens: Vec<WlOutputInfo>,
    use_curor: bool,
}

#[derive(Debug, Clone)]
pub enum CopySelect {
    Window { index: usize, show_cursor: bool },
    Screen { index: usize, show_cursor: bool },
    All,
    Slurp,
    Cancel,
}

#[derive(Debug, Clone)]
pub struct TopLevelInfo {
    pub top_level: TopLevel,
    pub image: Option<image::Handle>,
}

#[derive(Debug, Clone)]
pub struct WlOutputInfo {
    pub output: OutputInfo,
    pub image: Option<image::Handle>,
}

#[derive(Debug, Clone)]
pub enum ShowMode {
    Screens,
    Windows,
    Others,
}

#[to_layer_message(multi)]
#[derive(Debug, Clone)]
pub enum Message {
    ImageCopyOpen {
        top_levels: Vec<TopLevelInfo>,
        screens: Vec<WlOutputInfo>,
    },
    ScreenCastOpen {
        top_levels: Vec<TopLevelInfo>,
        screens: Vec<WlOutputInfo>,
    },
    #[allow(unused)]
    Selected {
        id: iced::window::Id,
        use_curor: bool,
        select: CopySelect,
    },
    ShowModeChange(ShowMode),
    ReadyShoot(Sender<CopySelect>),
    ReadyCast(Sender<CopySelect>),
    ToggleCursor(bool),
}

impl AreaSelectorGUI {
    fn toplevel_preview(
        &self,
        id: iced::window::Id,
        index: usize,
        info: &TopLevelInfo,
    ) -> Element<'_, Message> {
        let button_context: Element<Message> = match &info.image {
            Some(handle) => column![
                text(info.top_level.id_and_title())
                    .center()
                    .width(Length::Fill),
                image(handle).width(Length::Fill)
            ]
            .into(),
            None => text(info.top_level.id_and_title())
                .center()
                .width(Length::Fill)
                .into(),
        };
        button(button_context)
            .on_press(Message::Selected {
                id,
                select: CopySelect::Window {
                    index,
                    show_cursor: self.use_curor,
                },
                use_curor: self.use_curor,
            })
            .style(button::subtle)
            .into()
    }
    fn output_preview<'a>(
        &'a self,
        id: iced::window::Id,
        index: usize,
        info: &'a WlOutputInfo,
    ) -> Element<'a, Message> {
        let button_context: Element<'a, Message> = match &info.image {
            Some(handle) => column![
                text(&info.output.name).center().width(Length::Fill),
                image(handle).width(Length::Fill)
            ]
            .into(),
            None => text(&info.output.name).center().width(Length::Fill).into(),
        };
        button(button_context)
            .on_press(Message::Selected {
                id,
                select: CopySelect::Screen {
                    index,
                    show_cursor: self.use_curor,
                },
                use_curor: self.use_curor,
            })
            .style(button::subtle)
            .into()
    }
    fn selector(&self) -> Row<'_, Message> {
        let mut button_list = vec![];
        if self.gui_mode == GuiMode::ScreenShot {
            button_list.push(
                button(text("Others").center().width(Length::Fill))
                    .on_press_maybe(if self.gui_mode == GuiMode::ScreenShot {
                        Some(Message::ShowModeChange(ShowMode::Others))
                    } else {
                        None
                    })
                    .width(Length::Fill)
                    .style(if self.mode == ViewMode::Others {
                        button::primary
                    } else {
                        button::secondary
                    })
                    .into(),
            );
        }
        button_list.append(&mut vec![
            button(text("Screen").center().width(Length::Fill))
                .on_press(Message::ShowModeChange(ShowMode::Screens))
                .width(Length::Fill)
                .style(if self.mode == ViewMode::Screens {
                    button::primary
                } else {
                    button::secondary
                })
                .into(),
            button(text("Window").center().width(Length::Fill))
                .on_press_maybe(if self.toplevel_capture_support {
                    Some(Message::ShowModeChange(ShowMode::Windows))
                } else {
                    None
                })
                .width(Length::Fill)
                .style(if self.mode == ViewMode::Windows {
                    button::primary
                } else {
                    button::secondary
                })
                .into(),
        ]);
        Row::from_vec(button_list)
            .align_y(Alignment::Center)
            .spacing(10)
            .padding(20)
            .width(Length::Fill)
    }
    fn new(toplevel_capture_support: bool) -> Self {
        Self {
            gui_mode: GuiMode::ScreenShot,
            mode: ViewMode::Others,
            window_show: false,
            toplevel_capture_support,
            sender: None,
            sender_cast: None,
            toplevels: Vec::new(),
            screens: Vec::new(),
            use_curor: false,
        }
    }

    fn namespace() -> String {
        String::from("xdg-desktop-protal-luminous")
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::ShowModeChange(ShowMode::Screens) => {
                self.mode = ViewMode::Screens;
                Task::none()
            }
            Message::ShowModeChange(ShowMode::Windows) => {
                self.mode = ViewMode::Windows;
                Task::none()
            }
            Message::ShowModeChange(ShowMode::Others) => {
                self.mode = ViewMode::Others;
                Task::none()
            }
            Message::Selected { id, select, .. } => {
                use iced_runtime::Action;
                use iced_runtime::window::Action as WindowAction;
                match self.gui_mode {
                    GuiMode::ScreenCast => {
                        let _ = self.sender_cast.as_mut().unwrap().try_send(select);
                    }
                    GuiMode::ScreenShot => {
                        let _ = self.sender.as_mut().unwrap().try_send(select);
                    }
                }

                self.window_show = false;
                iced_runtime::task::effect(Action::Window(WindowAction::Close(id)))
            }

            Message::ImageCopyOpen {
                top_levels: toplevels,
                screens,
            } => {
                if self.window_show {
                    let _ = self.sender.as_mut().unwrap().try_send(CopySelect::Cancel);
                    return Task::none();
                }
                if self.gui_mode != GuiMode::ScreenShot {
                    self.mode = ViewMode::Others;
                }
                self.gui_mode = GuiMode::ScreenShot;
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
            Message::ScreenCastOpen {
                top_levels: toplevels,
                screens,
            } => {
                if self.window_show {
                    let _ = self
                        .sender_cast
                        .as_mut()
                        .unwrap()
                        .try_send(CopySelect::Cancel);
                    return Task::none();
                }
                if self.gui_mode == GuiMode::ScreenShot {
                    self.mode = ViewMode::Screens;
                }
                self.gui_mode = GuiMode::ScreenCast;
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
            Message::ReadyCast(sender) => {
                self.sender_cast = Some(sender);
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
        let selector = self.selector();

        let content: Element<'_, Message> = match self.mode {
            ViewMode::Screens => scrollable(
                column(
                    self.screens
                        .iter()
                        .enumerate()
                        .map(|(index, info)| self.output_preview(id, index, info)),
                )
                .spacing(10),
            )
            .height(Length::Fill)
            .into(),
            ViewMode::Windows => scrollable(
                column(
                    self.toplevels
                        .iter()
                        .enumerate()
                        .map(|(index, info)| self.toplevel_preview(id, index, info)),
                )
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
                let (sender_cast, receiver_cast) = channel(100);
                let _ = output.send(Message::ReadyShoot(sender)).await;
                let _ = output.send(Message::ReadyCast(sender_cast)).await;

                let _ = crate::backend::backend(output, receiver, receiver_cast).await;
            })
        })
    }
}
