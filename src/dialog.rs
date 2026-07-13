use std::collections::VecDeque;

use iced::futures::channel::mpsc::{Sender, UnboundedSender};
use iced::widget::{
    Row, Space, button, checkbox, column, container, grid, image, row, scrollable, text,
};
use iced::{Alignment, Element, Length, Task};
use iced_layershell::daemon;
use iced_layershell::reexport::{
    Anchor, KeyboardInteractivity, NewLayerShellSettings, OutputOption,
};
use iced_layershell::settings::{LayerShellSettings, StartMode};
use iced_layershell::to_layer_message;

use libwayshot::output::OutputInfo;
use libwayshot::region::TopLevel;

const BACKGROUND_PROMPT_QUEUE_CAPACITY: usize = 8;
const BACKGROUND_PROMPT_TOMBSTONE_CAPACITY: usize = 64;

pub fn dialog(toplevel_capture_support: bool) -> Result<(), iced_layershell::Error> {
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
        exclusive_zone: 0,
        anchor: Anchor::Bottom | Anchor::Left | Anchor::Right | Anchor::Top,
        keyboard_interactivity: KeyboardInteractivity::OnDemand,
        start_mode: StartMode::Background,
        ..Default::default()
    })
    .subscription(AreaSelectorGUI::subscription)
    .theme(AreaSelectorGUI::theme)
    .run()
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum GuiMode {
    ScreenCast,
    #[default]
    ScreenShot,
    PermissionPrompt,
    BackgroundPrompt,
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
    window_id: Option<iced::window::Id>,
    toplevel_capture_support: bool,
    sender: Option<Sender<CopySelect>>,
    sender_cast: Option<Sender<CopySelect>>,
    sender_background: Option<UnboundedSender<CopySelect>>,
    toplevels: Vec<TopLevelInfo>,
    screens: Vec<WlOutputInfo>,
    use_cursor: bool,
    prompt_text: Option<String>,
    active_background_handle: Option<String>,
    background_queue: VecDeque<BackgroundPromptRequest>,
    tombstoned_background_handles: VecDeque<String>,
}

#[derive(Debug, Clone)]
struct BackgroundPromptRequest {
    handle: String,
    app_id: String,
    name: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CopySelect {
    Window { index: usize, show_cursor: bool },
    Screen { index: usize, show_cursor: bool },
    All,
    Slurp,
    Cancel,
    Permission(bool),
    BackgroundPermission { handle: String, result: u32 },
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
        show_cursor: bool,
    },
    Selected {
        id: iced::window::Id,
        select: CopySelect,
    },
    ShowModeChange(ShowMode),
    ReadyShoot(Sender<CopySelect>),
    ReadyCast(Sender<CopySelect>),
    ReadyBackground(UnboundedSender<CopySelect>),
    ToggleCursor(bool),
    PermissionDialog(String),
    BackgroundPrompt {
        handle: String,
        app_id: String,
        name: String,
    },
    CloseBackgroundPrompt {
        handle: String,
    },
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
                    show_cursor: self.use_cursor,
                },
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
                    show_cursor: self.use_cursor,
                },
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
            window_id: None,
            toplevel_capture_support,
            sender: None,
            sender_cast: None,
            sender_background: None,
            toplevels: Vec::new(),
            screens: Vec::new(),
            use_cursor: false,
            prompt_text: None,
            active_background_handle: None,
            background_queue: VecDeque::new(),
            tombstoned_background_handles: VecDeque::new(),
        }
    }

    fn namespace() -> String {
        String::from("osk")
    }

    fn open_background_prompt(
        &mut self,
        handle: String,
        app_id: String,
        name: String,
    ) -> Task<Message> {
        self.window_show = true;
        self.gui_mode = GuiMode::BackgroundPrompt;
        self.active_background_handle = Some(handle);
        let app_name = if name.is_empty() { app_id } else { name };
        self.prompt_text = Some(format!(
            "Allow '{}' to keep running in the background?",
            app_name
        ));
        let id = iced::window::Id::unique();
        self.window_id = Some(id);
        Task::done(Message::NewLayerShell {
            settings: NewLayerShellSettings {
                size: Some((360, 128)),
                anchor: Anchor::Top | Anchor::Bottom,
                keyboard_interactivity: KeyboardInteractivity::OnDemand,
                output_option: OutputOption::Active,
                ..Default::default()
            },
            id,
        })
    }

    fn show_next_background_prompt(&mut self) -> Task<Message> {
        if self.window_show {
            return Task::none();
        }

        let Some(request) = self.background_queue.pop_front() else {
            return Task::none();
        };

        self.open_background_prompt(request.handle, request.app_id, request.name)
    }

    fn send_background_response(&self, select: CopySelect) {
        let CopySelect::BackgroundPermission { handle, result } = &select else {
            return;
        };
        let handle = handle.clone();
        let result = *result;

        let Some(sender) = &self.sender_background else {
            tracing::warn!(
                "Cannot deliver background permission result {result} for {handle}: response channel is not ready"
            );
            return;
        };

        if let Err(e) = sender.unbounded_send(select) {
            tracing::warn!(
                "Cannot deliver background permission result {result} for {handle}: receiver is gone: {e}"
            );
        }
    }

    fn tombstone_background_handle(&mut self, handle: String) {
        if self
            .tombstoned_background_handles
            .iter()
            .any(|tombstone| tombstone == &handle)
        {
            return;
        }

        if self.tombstoned_background_handles.len() >= BACKGROUND_PROMPT_TOMBSTONE_CAPACITY {
            self.tombstoned_background_handles.pop_front();
        }
        self.tombstoned_background_handles.push_back(handle);
    }

    fn consume_background_tombstone(&mut self, handle: &str) -> bool {
        let Some(index) = self
            .tombstoned_background_handles
            .iter()
            .position(|tombstone| tombstone == handle)
        else {
            return false;
        };

        self.tombstoned_background_handles.remove(index);
        true
    }

    fn close_window_and_show_next_background_prompt(
        &mut self,
        id: iced::window::Id,
    ) -> Task<Message> {
        use iced_runtime::Action;
        use iced_runtime::window::Action as WindowAction;

        let close_task = iced_runtime::task::effect(Action::Window(WindowAction::Close(id)));
        let next_prompt_task = self.show_next_background_prompt();
        Task::batch([close_task, next_prompt_task])
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
            Message::Selected { id, select } => {
                if self.window_id != Some(id) {
                    return Task::none();
                }

                match self.gui_mode {
                    GuiMode::ScreenCast => {
                        let _ = self.sender_cast.as_mut().unwrap().try_send(select);
                    }
                    GuiMode::BackgroundPrompt => match &select {
                        CopySelect::BackgroundPermission { handle, .. }
                            if self.active_background_handle.as_ref() == Some(handle) =>
                        {
                            self.send_background_response(select);
                        }
                        _ => return Task::none(),
                    },
                    GuiMode::ScreenShot | GuiMode::PermissionPrompt => {
                        if matches!(select, CopySelect::BackgroundPermission { .. }) {
                            return Task::none();
                        }
                        let _ = self.sender.as_mut().unwrap().try_send(select);
                    }
                }

                self.window_show = false;
                self.window_id = None;
                if self.gui_mode == GuiMode::BackgroundPrompt {
                    self.gui_mode = GuiMode::ScreenShot;
                    self.prompt_text = None;
                    self.active_background_handle = None;
                }
                self.close_window_and_show_next_background_prompt(id)
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
                let id = iced::window::Id::unique();
                self.window_id = Some(id);
                Task::done(Message::NewLayerShell {
                    settings: NewLayerShellSettings {
                        exclusive_zone: None,
                        anchor: Anchor::Right | Anchor::Top | Anchor::Left | Anchor::Bottom,
                        margin: Some((300, 300, 300, 300)),
                        keyboard_interactivity: KeyboardInteractivity::OnDemand,
                        output_option: OutputOption::Active,
                        ..Default::default()
                    },
                    id,
                })
            }
            Message::ScreenCastOpen {
                top_levels: toplevels,
                screens,
                show_cursor,
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
                self.use_cursor = show_cursor;
                self.gui_mode = GuiMode::ScreenCast;
                self.window_show = true;
                self.toplevels = toplevels;
                self.screens = screens;
                let id = iced::window::Id::unique();
                self.window_id = Some(id);
                Task::done(Message::NewLayerShell {
                    settings: NewLayerShellSettings {
                        exclusive_zone: None,
                        anchor: Anchor::Right | Anchor::Top | Anchor::Left | Anchor::Bottom,
                        margin: Some((300, 500, 300, 500)),
                        keyboard_interactivity: KeyboardInteractivity::OnDemand,
                        output_option: OutputOption::Active,
                        ..Default::default()
                    },
                    id,
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
            Message::ReadyBackground(sender) => {
                self.sender_background = Some(sender);
                Task::none()
            }
            Message::ToggleCursor(cursor) => {
                self.use_cursor = cursor;
                Task::none()
            }
            Message::PermissionDialog(message) => {
                if self.window_show {
                    let _ = self.sender.as_mut().unwrap().try_send(CopySelect::Cancel);
                    return Task::none();
                }
                self.window_show = true;
                self.gui_mode = GuiMode::PermissionPrompt;
                self.prompt_text = Some(message);
                let id = iced::window::Id::unique();
                self.window_id = Some(id);
                Task::done(Message::NewLayerShell {
                    settings: NewLayerShellSettings {
                        size: Some((256, 100)),
                        anchor: Anchor::Top | Anchor::Bottom,
                        keyboard_interactivity: KeyboardInteractivity::OnDemand,
                        output_option: OutputOption::Active,
                        ..Default::default()
                    },
                    id,
                })
            }
            Message::BackgroundPrompt {
                handle,
                app_id,
                name,
            } => {
                if self.consume_background_tombstone(&handle) {
                    return Task::none();
                }

                if self.window_show {
                    if self.background_queue.len() >= BACKGROUND_PROMPT_QUEUE_CAPACITY {
                        self.send_background_response(CopySelect::BackgroundPermission {
                            handle,
                            result: 2,
                        });
                    } else {
                        self.background_queue.push_back(BackgroundPromptRequest {
                            handle,
                            app_id,
                            name,
                        });
                    }
                    return Task::none();
                }
                self.open_background_prompt(handle, app_id, name)
            }
            Message::CloseBackgroundPrompt { handle } => {
                if self.gui_mode != GuiMode::BackgroundPrompt
                    || self.active_background_handle.as_ref() != Some(&handle)
                {
                    let previous_queue_len = self.background_queue.len();
                    self.background_queue
                        .retain(|request| request.handle != handle);
                    if self.background_queue.len() == previous_queue_len {
                        self.tombstone_background_handle(handle);
                    }
                    return Task::none();
                }

                self.window_show = false;
                self.gui_mode = GuiMode::ScreenShot;
                self.prompt_text = None;
                self.active_background_handle = None;

                if let Some(id) = self.window_id.take() {
                    self.close_window_and_show_next_background_prompt(id)
                } else {
                    self.show_next_background_prompt()
                }
            }
            _ => unreachable!(),
        }
    }

    fn view_permission_prompt(&self, id: iced::window::Id) -> Element<'_, Message> {
        column![
            container(text(self.prompt_text.as_ref().unwrap()))
                .center_x(Length::Fill)
                .center_y(Length::Fill)
                .height(Length::Fill),
            Space::new().height(Length::Fill),
            row![
                button("No")
                    .style(button::text)
                    .on_press(Message::Selected {
                        id,
                        select: CopySelect::Permission(false)
                    })
                    .width(Length::Fill),
                button("Yes")
                    .on_press(Message::Selected {
                        id,
                        select: CopySelect::Permission(true)
                    })
                    .width(Length::Fill)
            ]
            .padding(2.)
            .spacing(5.)
            .width(Length::Fill)
        ]
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }

    fn view_background_prompt(&self, id: iced::window::Id) -> Element<'_, Message> {
        let handle = self.active_background_handle.clone().unwrap_or_default();

        column![
            container(text(self.prompt_text.as_ref().unwrap()))
                .center_x(Length::Fill)
                .center_y(Length::Fill)
                .height(Length::Fill),
            Space::new().height(Length::Fill),
            row![
                button("Allow")
                    .on_press(Message::Selected {
                        id,
                        select: CopySelect::BackgroundPermission {
                            handle: handle.clone(),
                            result: 1
                        }
                    })
                    .width(Length::Fill),
                button("Allow once")
                    .on_press(Message::Selected {
                        id,
                        select: CopySelect::BackgroundPermission {
                            handle: handle.clone(),
                            result: 2
                        }
                    })
                    .width(Length::Fill),
                button("Deny")
                    .style(button::text)
                    .on_press(Message::Selected {
                        id,
                        select: CopySelect::BackgroundPermission { handle, result: 0 }
                    })
                    .width(Length::Fill),
            ]
            .padding(2.)
            .spacing(5.)
            .width(Length::Fill)
        ]
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }

    fn view(&self, id: iced::window::Id) -> Element<'_, Message> {
        if self.gui_mode == GuiMode::PermissionPrompt {
            return self.view_permission_prompt(id);
        }
        if self.gui_mode == GuiMode::BackgroundPrompt {
            return self.view_background_prompt(id);
        }

        let selector = self.selector();

        let content: Element<'_, Message> = match self.mode {
            ViewMode::Screens => scrollable(
                grid(
                    self.screens
                        .iter()
                        .enumerate()
                        .map(|(index, info)| self.output_preview(id, index, info)),
                )
                .columns(2)
                .spacing(10),
            )
            .height(Length::Fill)
            .into(),
            ViewMode::Windows => scrollable(
                grid(
                    self.toplevels
                        .iter()
                        .enumerate()
                        .map(|(index, info)| self.toplevel_preview(id, index, info)),
                )
                .columns(3)
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
                checkbox(self.use_cursor)
                    .label("use_cursor")
                    .on_toggle_maybe(if self.gui_mode == GuiMode::ScreenShot {
                        Some(Message::ToggleCursor)
                    } else {
                        None
                    })
            )
            .center_y(Length::Fill),
            Space::new().width(Length::Fixed(2.)),
            button(text("Cancel")).on_press(Message::Selected {
                id,
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
                use iced::futures::channel::mpsc::{channel, unbounded};
                use iced::futures::sink::SinkExt;
                let (sender, receiver) = channel(100);
                let (sender_cast, receiver_cast) = channel(100);
                let (sender_background, receiver_background) = unbounded();
                let _ = output.send(Message::ReadyShoot(sender)).await;
                let _ = output.send(Message::ReadyCast(sender_cast)).await;
                let _ = output
                    .send(Message::ReadyBackground(sender_background))
                    .await;

                let _ =
                    crate::backend::backend(output, receiver, receiver_cast, receiver_background)
                        .await;
            })
        })
    }
    fn theme(&self, _id: iced::window::Id) -> Option<iced::Theme> {
        if self.gui_mode == GuiMode::PermissionPrompt || self.gui_mode == GuiMode::BackgroundPrompt
        {
            return Some(iced::Theme::TokyoNight);
        }
        None
    }
}
