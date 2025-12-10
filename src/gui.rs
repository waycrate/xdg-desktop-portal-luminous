use iced::widget::{button, column, row, text};
use iced::{Alignment, Element, Length, Task as Command};
use iced_layershell::daemon;
use iced_layershell::reexport::{Anchor, KeyboardInteractivity};
use iced_layershell::settings::{LayerShellSettings, Settings, StartMode};
use iced_layershell::to_layer_message;

pub fn gui() -> Result<(), iced_layershell::Error> {
    unsafe { std::env::set_var("RUST_LOG", "xdg-desktop-protal-luminous=info") }
    tracing_subscriber::fmt().init();
    tracing::info!("luminous Start");
    daemon(
        || AreaSelectorGUI::new(),
        AreaSelectorGUI::namespace,
        AreaSelectorGUI::update,
        AreaSelectorGUI::view,
    )
    .title(AreaSelectorGUI::title)
    .settings(Settings {
        layer_settings: LayerShellSettings {
            size: Some((400, 400)),
            exclusive_zone: 0,
            anchor: Anchor::Bottom | Anchor::Left | Anchor::Right | Anchor::Top,
            keyboard_interactivity: KeyboardInteractivity::OnDemand,
            start_mode: StartMode::Background,
            ..Default::default()
        },
        ..Default::default()
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
}

#[to_layer_message(multi)]
#[derive(Debug, Clone)]
enum Message {
    ShowScreens,
    ShowWindows,
    ScreenSelected(u32),
    WindowSelected(u32),
}

impl AreaSelectorGUI {
    fn new() -> Self {
        Self {
            mode: ViewMode::Screens,
        }
    }

    fn title(&self, _id: iced::window::Id) -> Option<String> {
        None
    }

    fn namespace() -> String {
        String::from("Area Selector")
    }

    fn update(&mut self, message: Message) -> Command<Message> {
        match message {
            Message::ShowScreens => {
                self.mode = ViewMode::Screens;
            }
            Message::ShowWindows => {
                self.mode = ViewMode::Windows;
            }
            Message::ScreenSelected(id) => {
                println!("Screen {} selected!", id);
            }
            Message::WindowSelected(id) => {
                println!("Window {} selected!", id);
            }
            _ => {}
        }
        Command::none()
    }

    fn view(&self, _id: iced::window::Id) -> Element<'_, Message> {
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
                .on_press(Message::ShowWindows)
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
            ViewMode::Screens => column![
                button(text("Screen 1").center().width(Length::Fill))
                    .on_press(Message::ScreenSelected(1))
                    .width(Length::Fill)
                    .style(button::subtle),
                button(text("Screen 2").center().width(Length::Fill))
                    .on_press(Message::ScreenSelected(2))
                    .width(Length::Fill)
                    .style(button::subtle),
                button(text("Screen 3").center().width(Length::Fill))
                    .on_press(Message::ScreenSelected(3))
                    .width(Length::Fill)
                    .style(button::subtle),
            ]
            .spacing(10)
            .into(),
            ViewMode::Windows => column![
                button(text("Window 1").center().width(Length::Fill))
                    .on_press(Message::WindowSelected(1))
                    .width(Length::Fill)
                    .style(button::subtle),
                button(text("Window 2").center().width(Length::Fill))
                    .on_press(Message::WindowSelected(2))
                    .width(Length::Fill)
                    .style(button::subtle),
                button(text("Window 3").center().width(Length::Fill))
                    .on_press(Message::WindowSelected(3))
                    .width(Length::Fill)
                    .style(button::subtle),
            ]
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
            iced::stream::channel(100, |_| async move {
                let _ = crate::backend::backend().await;
            })
        })
    }
}
