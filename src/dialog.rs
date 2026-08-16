use std::collections::VecDeque;

use iced::futures::channel::mpsc::{Sender, UnboundedSender};
use iced::widget::{
    Row, Space, button, checkbox, column, container, grid, image, row, rule, scrollable, text,
};
use iced::{
    Alignment, Background, Border, Color, ContentFit, Element, Font, Length, Pixels, Shadow, Task,
    Vector,
};
use iced_layershell::daemon;
use iced_layershell::reexport::{
    Anchor, KeyboardInteractivity, NewLayerShellSettings, OutputOption,
};
use iced_layershell::settings::{LayerShellSettings, StartMode};
use iced_layershell::to_layer_message;

use libwayshot::output::OutputInfo;
use libwayshot::region::TopLevel;

use crate::settings::SettingsConfig;

const BACKGROUND_PROMPT_QUEUE_CAPACITY: usize = 8;
const BACKGROUND_PROMPT_TOMBSTONE_CAPACITY: usize = 64;
const CHOOSER_WIDTH: u32 = 1000;
const CHOOSER_HEIGHT: u32 = 620;
const CHOOSER_SHADOW_MARGIN: u32 = 24;
const PERMISSION_DIALOG_WIDTH: u32 = 420;
const PERMISSION_DIALOG_HEIGHT: u32 = 200;
const PERMISSION_DIALOG_SHADOW_MARGIN: u32 = 16;
const PREVIEW_BUTTON_HEIGHT: f32 = 320.0;
const PREVIEW_BUTTON_PADDING: u16 = 8;
const PREVIEW_BUTTON_LINE_HEIGHT: f32 = 17.0;
const FOOTER_HEIGHT: f32 = 81.0;
const FOOTER_BOX_HEIGHT: f32 = 33.0;

const ACCENT: Color = Color::from_rgb8(56, 132, 228);
const DARK_ACCENT: Color = Color::from_rgb8(21, 83, 158);

const FONT_MEDIUM: Font = Font {
    weight: iced::font::Weight::Medium,
    ..Font::DEFAULT
};
const FONT_SEMIBOLD: Font = Font {
    weight: iced::font::Weight::Semibold,
    ..Font::DEFAULT
};

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
        anchor: Anchor::all(),
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
    prefers_dark: bool,
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
    ColorSchemeChanged(bool),
}

fn dialog_style(outlined: bool) -> impl Fn(&iced::Theme) -> container::Style + Copy {
    move |theme| {
        let palette = theme.extended_palette();
        let base_palette = if palette.is_dark {
            iced::theme::Palette::DARK
        } else {
            iced::theme::Palette::LIGHT
        };

        container::Style {
            background: Some(Background::Color(base_palette.background)),
            text_color: Some(base_palette.text),
            border: Border {
                color: if outlined {
                    palette.background.strong.color
                } else {
                    Color::TRANSPARENT
                },
                width: if outlined { 1.0 } else { 0.0 },
                radius: 12.0.into(),
            },
            shadow: Shadow {
                color: Color::from_rgba8(0, 0, 0, if outlined { 0.15 } else { 0.12 }),
                offset: Vector::new(0.0, 4.0),
                blur_radius: if outlined { 12.0 } else { 16.0 },
            },
            ..container::Style::default()
        }
    }
}

fn tab_bar_style(theme: &iced::Theme) -> container::Style {
    let mut style = container::rounded_box(theme);
    style.border.radius = 8.0.into();
    style
}

fn tab_style(selected: bool) -> impl Fn(&iced::Theme, button::Status) -> button::Style + Copy {
    move |theme, status| {
        let mut style = if selected {
            button::primary(theme, status)
        } else {
            button::text(theme, status)
        };
        style.border.radius = 6.0.into();
        style
    }
}

fn bordered_button_style(theme: &iced::Theme, status: button::Status) -> button::Style {
    let button_theme = if theme.extended_palette().is_dark {
        iced::Theme::Dark
    } else {
        iced::Theme::Light
    };
    let mut style = button::background(&button_theme, status);
    style.border = Border {
        color: theme.extended_palette().background.strong.color,
        width: 1.0,
        radius: 8.0.into(),
    };
    style
}

fn primary_button_style(theme: &iced::Theme, status: button::Status) -> button::Style {
    let mut style = button::primary(theme, status);
    style.border.radius = 8.0.into();
    style
}

fn divider() -> Element<'static, Message> {
    rule::horizontal(1).style(rule::weak).into()
}

fn chooser_layer_settings() -> NewLayerShellSettings {
    NewLayerShellSettings {
        size: Some((
            CHOOSER_WIDTH + CHOOSER_SHADOW_MARGIN * 2,
            CHOOSER_HEIGHT + CHOOSER_SHADOW_MARGIN * 2,
        )),
        exclusive_zone: None,
        anchor: Anchor::all(),
        keyboard_interactivity: KeyboardInteractivity::OnDemand,
        output_option: OutputOption::Active,
        ..Default::default()
    }
}

fn permission_layer_settings() -> NewLayerShellSettings {
    NewLayerShellSettings {
        size: Some((
            PERMISSION_DIALOG_WIDTH + PERMISSION_DIALOG_SHADOW_MARGIN * 2,
            PERMISSION_DIALOG_HEIGHT + PERMISSION_DIALOG_SHADOW_MARGIN * 2,
        )),
        exclusive_zone: None,
        anchor: Anchor::Top | Anchor::Bottom,
        keyboard_interactivity: KeyboardInteractivity::OnDemand,
        output_option: OutputOption::Active,
        ..Default::default()
    }
}

fn dialog_theme(prefers_dark: bool) -> iced::Theme {
    let base_palette = if prefers_dark {
        iced::theme::Palette::DARK
    } else {
        iced::theme::Palette::LIGHT
    };
    let widget_palette = iced::theme::Palette {
        primary: if prefers_dark { DARK_ACCENT } else { ACCENT },
        ..base_palette
    };
    let mut widget_colors = iced::theme::palette::Extended::generate(widget_palette);
    widget_colors.background.base.color = Color::TRANSPARENT;

    iced::Theme::custom_with_fn(
        "Luminous dialogs",
        iced::theme::Palette {
            background: Color::TRANSPARENT,
            ..widget_palette
        },
        move |_| widget_colors,
    )
}

impl AreaSelectorGUI {
    fn preview<'a>(handle: Option<&'a image::Handle>) -> Element<'a, Message> {
        let preview: Element<'a, Message> = match handle {
            Some(handle) => image(handle)
                .width(Length::Fill)
                .height(Length::Fill)
                .content_fit(ContentFit::Contain)
                .into(),
            None => Space::new().width(Length::Fill).height(Length::Fill).into(),
        };

        container(preview)
            .width(Length::Fill)
            .height(Length::Fixed(161.0))
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .style(container::rounded_box)
            .into()
    }

    fn toplevel_preview<'a>(
        &'a self,
        id: iced::window::Id,
        index: usize,
        info: &'a TopLevelInfo,
    ) -> Element<'a, Message> {
        let button_context = column![
            Self::preview(info.image.as_ref()),
            text(info.top_level.id_and_title())
                .center()
                .width(Length::Fill)
                .size(14)
                .line_height(Pixels(17.0))
                .font(FONT_MEDIUM)
        ]
        .spacing(12)
        .width(Length::Fill)
        .align_x(Alignment::Center);

        button(button_context)
            .on_press(Message::Selected {
                id,
                select: CopySelect::Window {
                    index,
                    show_cursor: self.use_cursor,
                },
            })
            .width(Length::Fill)
            .height(Length::Fixed(PREVIEW_BUTTON_HEIGHT))
            .padding(PREVIEW_BUTTON_PADDING)
            .style(bordered_button_style)
            .into()
    }

    fn output_preview<'a>(
        &'a self,
        id: iced::window::Id,
        index: usize,
        info: &'a WlOutputInfo,
    ) -> Element<'a, Message> {
        let select = CopySelect::Screen {
            index,
            show_cursor: self.use_cursor,
        };

        if info.image.is_none() {
            return self.option_card(id, &info.output.name, select);
        }

        let button_context = column![
            Self::preview(info.image.as_ref()),
            text(&info.output.name)
                .center()
                .width(Length::Fill)
                .size(14)
                .line_height(Pixels(PREVIEW_BUTTON_LINE_HEIGHT))
                .font(FONT_MEDIUM)
        ]
        .spacing(12)
        .width(Length::Fill)
        .align_x(Alignment::Center);

        button(button_context)
            .on_press(Message::Selected { id, select })
            .width(Length::Fill)
            .height(Length::Fixed(PREVIEW_BUTTON_HEIGHT))
            .padding(PREVIEW_BUTTON_PADDING)
            .style(bordered_button_style)
            .into()
    }

    fn option_card<'a>(
        &self,
        id: iced::window::Id,
        label: &'a str,
        select: CopySelect,
    ) -> Element<'a, Message> {
        button(
            container(
                text(label)
                    .center()
                    .width(Length::Fill)
                    .size(14)
                    .line_height(Pixels(PREVIEW_BUTTON_LINE_HEIGHT))
                    .font(FONT_MEDIUM),
            )
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .width(Length::Fill)
            .height(Length::Fill),
        )
        .on_press(Message::Selected { id, select })
        .width(Length::Fill)
        .height(Length::Fixed(PREVIEW_BUTTON_HEIGHT))
        .padding(PREVIEW_BUTTON_PADDING)
        .style(bordered_button_style)
        .into()
    }

    fn tab_button(
        &self,
        label: &'static str,
        selected: bool,
        on_press: Option<Message>,
    ) -> Element<'_, Message> {
        button(
            text(label)
                .center()
                .width(Length::Fill)
                .size(14)
                .line_height(Pixels(17.0))
                .font(FONT_MEDIUM),
        )
        .on_press_maybe(on_press)
        .width(Length::Fill)
        .height(Length::Fixed(29.0))
        .padding([6, 8])
        .style(tab_style(selected))
        .into()
    }

    fn selector(&self) -> Element<'_, Message> {
        let mut button_list = vec![];
        if self.gui_mode == GuiMode::ScreenShot {
            button_list.push(self.tab_button(
                "Options",
                self.mode == ViewMode::Others,
                Some(Message::ShowModeChange(ShowMode::Others)),
            ));
        }
        button_list.append(&mut vec![
            self.tab_button(
                "Screen",
                self.mode == ViewMode::Screens,
                Some(Message::ShowModeChange(ShowMode::Screens)),
            ),
            self.tab_button(
                "Window",
                self.mode == ViewMode::Windows,
                if self.toplevel_capture_support {
                    Some(Message::ShowModeChange(ShowMode::Windows))
                } else {
                    None
                },
            ),
        ]);

        container(
            Row::from_vec(button_list)
                .align_y(Alignment::Center)
                .spacing(4)
                .width(Length::Fill),
        )
        .padding(6)
        .width(Length::Fill)
        .height(Length::Fixed(41.0))
        .style(tab_bar_style)
        .into()
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
            prefers_dark: SettingsConfig::config_from_file().prefers_dark(),
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
            settings: permission_layer_settings(),
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
                    settings: chooser_layer_settings(),
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
                    settings: chooser_layer_settings(),
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
                    settings: permission_layer_settings(),
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
            Message::ColorSchemeChanged(prefers_dark) => {
                self.prefers_dark = prefers_dark;
                Task::none()
            }
            _ => unreachable!(),
        }
    }

    fn view_prompt<'a>(&'a self, button_row: Element<'a, Message>) -> Element<'a, Message> {
        let dialog = container(
            column![
                text(self.prompt_text.as_deref().unwrap_or_default())
                    .width(Length::Fill)
                    .height(Length::Fixed(60.0))
                    .size(20)
                    .line_height(Pixels(24.0))
                    .font(FONT_SEMIBOLD),
                Space::new().height(Length::Fixed(16.0)),
                divider(),
                Space::new().height(Length::Fixed(15.0)),
                button_row,
            ]
            .width(Length::Fill)
            .height(Length::Fill),
        )
        .padding(24)
        .width(Length::Fixed(PERMISSION_DIALOG_WIDTH as f32))
        .height(Length::Fixed(PERMISSION_DIALOG_HEIGHT as f32))
        .style(dialog_style(false));

        container(dialog)
            .padding(PERMISSION_DIALOG_SHADOW_MARGIN as f32)
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .into()
    }

    fn view_permission_prompt(&self, id: iced::window::Id) -> Element<'_, Message> {
        let deny_button = button(
            text("Deny")
                .size(14)
                .line_height(Pixels(17.0))
                .font(FONT_MEDIUM),
        )
        .on_press(Message::Selected {
            id,
            select: CopySelect::Permission(false),
        })
        .height(Length::Fixed(33.0))
        .padding([8, 16])
        .style(bordered_button_style);

        let allow_button = button(
            text("Allow")
                .size(14)
                .line_height(Pixels(17.0))
                .font(FONT_MEDIUM),
        )
        .on_press(Message::Selected {
            id,
            select: CopySelect::Permission(true),
        })
        .height(Length::Fixed(33.0))
        .padding([8, 16])
        .style(primary_button_style);

        let button_row = row![Space::new().width(Length::Fill), deny_button, allow_button]
            .align_y(Alignment::Center)
            .spacing(10)
            .padding(10)
            .width(Length::Fill)
            .height(Length::Fixed(60.0));

        self.view_prompt(button_row.into())
    }

    fn view_background_prompt(&self, id: iced::window::Id) -> Element<'_, Message> {
        let handle = self.active_background_handle.clone().unwrap_or_default();

        let deny_button = button(
            text("Deny")
                .size(14)
                .line_height(Pixels(17.0))
                .font(FONT_MEDIUM),
        )
        .on_press(Message::Selected {
            id,
            select: CopySelect::BackgroundPermission {
                handle: handle.clone(),
                result: 0,
            },
        })
        .height(Length::Fixed(33.0))
        .padding([8, 16])
        .style(bordered_button_style);

        let allow_once_button = button(
            text("Allow once")
                .size(14)
                .line_height(Pixels(17.0))
                .font(FONT_MEDIUM),
        )
        .on_press(Message::Selected {
            id,
            select: CopySelect::BackgroundPermission {
                handle: handle.clone(),
                result: 2,
            },
        })
        .height(Length::Fixed(33.0))
        .padding([8, 16])
        .style(bordered_button_style);

        let allow_button = button(
            text("Allow")
                .size(14)
                .line_height(Pixels(17.0))
                .font(FONT_MEDIUM),
        )
        .on_press(Message::Selected {
            id,
            select: CopySelect::BackgroundPermission { handle, result: 1 },
        })
        .height(Length::Fixed(33.0))
        .padding([8, 16])
        .style(primary_button_style);

        let button_row = row![
            Space::new().width(Length::Fill),
            deny_button,
            allow_once_button,
            allow_button,
        ]
        .align_y(Alignment::Center)
        .spacing(10)
        .padding(10)
        .width(Length::Fill)
        .height(Length::Fixed(60.0));

        self.view_prompt(button_row.into())
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
                .spacing(12)
                .height(Length::Shrink),
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
                .columns(2)
                .spacing(12)
                .height(Length::Shrink),
            )
            .height(Length::Fill)
            .into(),
            ViewMode::Others => grid(vec![
                self.option_card(id, "Area Select", CopySelect::Slurp),
                self.option_card(id, "All Screens", CopySelect::All),
            ])
            .columns(2)
            .spacing(12)
            .height(Length::Shrink)
            .into(),
        };
        let content = container(content)
            .padding(10)
            .width(Length::Fill)
            .height(Length::Fill);

        let cursor_checkbox = checkbox(self.use_cursor)
            .label("Include cursor")
            .on_toggle_maybe(if self.gui_mode == GuiMode::ScreenShot {
                Some(Message::ToggleCursor)
            } else {
                None
            })
            .size(16)
            .spacing(8)
            .text_size(14)
            .font(FONT_MEDIUM);

        let cancel_button = button(
            text("Cancel")
                .size(14)
                .line_height(Pixels(17.0))
                .font(FONT_MEDIUM),
        )
        .on_press(Message::Selected {
            id,
            select: CopySelect::Cancel,
        })
        .height(Length::Fixed(33.0))
        .padding([8, 16])
        .style(bordered_button_style);

        let footer = container(
            row![
                cursor_checkbox,
                Space::new().width(Length::Fill),
                cancel_button
            ]
            .align_y(Alignment::Center)
            .spacing(10)
            .width(Length::Fill)
            .height(Length::Fixed(FOOTER_BOX_HEIGHT)),
        )
        .padding([24, 0])
        .width(Length::Fill)
        .height(Length::Fixed(FOOTER_HEIGHT));

        let title = if self.gui_mode == GuiMode::ScreenShot {
            "Take a Screenshot"
        } else {
            "Share Your Screen"
        };

        let dialog = container(
            column![
                text(title)
                    .width(Length::Fill)
                    .height(Length::Fixed(24.0))
                    .size(20)
                    .line_height(Pixels(24.0))
                    .font(FONT_SEMIBOLD),
                Space::new().height(Length::Fixed(16.0)),
                divider(),
                Space::new().height(Length::Fixed(15.0)),
                selector,
                Space::new().height(Length::Fixed(16.0)),
                content,
                Space::new().height(Length::Fixed(16.0)),
                divider(),
                Space::new().height(Length::Fixed(15.0)),
                footer,
            ]
            .width(Length::Fill)
            .height(Length::Fill),
        )
        .padding(24)
        .width(Length::Fixed(CHOOSER_WIDTH as f32))
        .height(Length::Fixed(CHOOSER_HEIGHT as f32))
        .style(dialog_style(true));

        container(dialog)
            .padding(CHOOSER_SHADOW_MARGIN as f32)
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x(Length::Fill)
            .center_y(Length::Fill)
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
        Some(dialog_theme(self.prefers_dark))
    }
}
