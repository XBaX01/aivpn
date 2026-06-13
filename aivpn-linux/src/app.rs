use iced::widget::{button, column, container, row, scrollable, text, text_input, Space};
use iced::{Alignment, Element, Length, Subscription, Task, Theme};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, BufReader};

use crate::vpn_manager::{find_client_binary, VpnStatus};

const MAX_LOG_LINES: usize = 200;

#[derive(Debug, Clone)]
pub enum Message {
    KeyInputChanged(String),
    Connect,
    Disconnect,
    StatusReceived(VpnStatus),
    LogLine(String),
    ClearLog,
}

pub struct App {
    key_input: String,
    status: VpnStatus,
    log_lines: Vec<String>,
    /// Set to Some(key) when actively connecting/connected; None when idle.
    connection_key: Option<String>,
    /// Shared handle so Disconnect can kill the process while the subscription owns it.
    child_handle: Arc<Mutex<Option<tokio::process::Child>>>,
}

impl App {
    pub fn new() -> (Self, Task<Message>) {
        (
            Self {
                key_input: String::new(),
                status: VpnStatus::Disconnected,
                log_lines: Vec::new(),
                connection_key: None,
                child_handle: Arc::new(Mutex::new(None)),
            },
            Task::none(),
        )
    }

    pub fn update(&mut self, msg: Message) -> Task<Message> {
        match msg {
            Message::KeyInputChanged(s) => {
                self.key_input = s;
            }

            Message::Connect => {
                let key = self.key_input.trim().to_string();
                if key.is_empty() {
                    self.push_log("Error: connection key is empty".to_string());
                    return Task::none();
                }
                self.connection_key = Some(key);
                self.status = VpnStatus::Connecting;
            }

            Message::Disconnect => {
                self.connection_key = None;
                // Kill the process owned by the subscription
                if let Ok(mut guard) = self.child_handle.lock() {
                    if let Some(child) = guard.as_mut() {
                        let _ = child.start_kill();
                    }
                    *guard = None;
                }
                self.status = VpnStatus::Disconnected;
                self.push_log("Disconnected".to_string());
            }

            Message::StatusReceived(s) => {
                self.status = s;
            }

            Message::LogLine(line) => {
                self.push_log(line);
            }

            Message::ClearLog => {
                self.log_lines.clear();
            }
        }
        Task::none()
    }

    pub fn view(&self) -> Element<Message> {
        let status_label = match &self.status {
            VpnStatus::Disconnected => text("● Disconnected").color([0.8, 0.2, 0.2]).size(15),
            VpnStatus::Connecting => text("◌ Connecting…").color([0.9, 0.7, 0.1]).size(15),
            VpnStatus::Connected { vpn_ip } => text(format!("● Connected  {vpn_ip}"))
                .color([0.2, 0.8, 0.3])
                .size(15),
            VpnStatus::Error(e) => text(format!("✗ {e}")).color([0.9, 0.2, 0.1]).size(15),
        };

        let key_row = row![
            text_input("Paste aivpn:// connection key…", &self.key_input)
                .on_input(Message::KeyInputChanged)
                .width(Length::Fill),
        ];

        let busy = matches!(
            self.status,
            VpnStatus::Connected { .. } | VpnStatus::Connecting
        );

        let action_btn = if busy {
            button("Disconnect").on_press(Message::Disconnect)
        } else {
            button("Connect").on_press(Message::Connect)
        };

        let btn_row = row![
            action_btn,
            Space::with_width(Length::Fill),
            button("Clear log").on_press(Message::ClearLog),
        ]
        .spacing(8)
        .align_y(Alignment::Center);

        let log_items: Vec<Element<Message>> = if self.log_lines.is_empty() {
            vec![text("No output yet").color([0.5, 0.5, 0.5]).into()]
        } else {
            self.log_lines
                .iter()
                .map(|l| text(l).size(12).into())
                .collect()
        };

        let log_box = scrollable(
            container(column(log_items).spacing(2))
                .padding(8)
                .width(Length::Fill),
        )
        .height(Length::Fill);

        container(
            column![
                status_label,
                Space::with_height(8),
                key_row,
                Space::with_height(6),
                btn_row,
                Space::with_height(8),
                log_box,
            ]
            .padding(16)
            .spacing(4),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }

    pub fn subscription(&self) -> Subscription<Message> {
        match &self.connection_key {
            Some(key) => {
                let key = key.clone();
                let child_handle = self.child_handle.clone();
                let stream = iced::stream::channel(64, move |mut sender| async move {
                    let binary = match find_client_binary() {
                        Ok(b) => b,
                        Err(e) => {
                            let _ = sender.try_send(Message::StatusReceived(VpnStatus::Error(e)));
                            return;
                        }
                    };

                    let mut child = match tokio::process::Command::new(&binary)
                        .args(["--connection-key", &key])
                        .stdout(std::process::Stdio::piped())
                        .stderr(std::process::Stdio::piped())
                        .spawn()
                    {
                        Ok(c) => c,
                        Err(e) => {
                            let _ = sender.try_send(Message::StatusReceived(VpnStatus::Error(
                                format!("Launch failed: {e}"),
                            )));
                            return;
                        }
                    };

                    let stdout = child.stdout.take().unwrap();
                    let stderr = child.stderr.take().unwrap();
                    *child_handle.lock().unwrap() = Some(child);
                    let _ = sender.try_send(Message::StatusReceived(VpnStatus::Connecting));

                    let mut out = BufReader::new(stdout).lines();
                    let mut err = BufReader::new(stderr).lines();

                    loop {
                        tokio::select! {
                            line = out.next_line() => match line {
                                Ok(Some(l)) => {
                                    if l.contains("Connected") || l.contains("TUN interface") {
                                        let ip = l.split_whitespace()
                                            .find(|t| t.contains('.') && t.contains('/'))
                                            .map(|s| s.to_string())
                                            .unwrap_or_default();
                                        let _ = sender.try_send(Message::StatusReceived(
                                            VpnStatus::Connected { vpn_ip: ip },
                                        ));
                                    }
                                    let _ = sender.try_send(Message::LogLine(l));
                                }
                                _ => break,
                            },
                            line = err.next_line() => match line {
                                Ok(Some(l)) => {
                                    let _ = sender.try_send(Message::LogLine(format!("[err] {l}")));
                                }
                                _ => break,
                            },
                        }
                    }

                    let _ = sender.try_send(Message::StatusReceived(VpnStatus::Disconnected));
                });
                Subscription::run_with_id("aivpn_worker", stream)
            }
            None => Subscription::none(),
        }
    }

    pub fn theme(&self) -> Theme {
        Theme::Dark
    }

    fn push_log(&mut self, line: String) {
        self.log_lines.push(line);
        if self.log_lines.len() > MAX_LOG_LINES {
            let excess = self.log_lines.len() - MAX_LOG_LINES;
            self.log_lines.drain(0..excess);
        }
    }
}
