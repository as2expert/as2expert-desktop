//! The AS2Expert desktop application: an Outlook-style client over the SDK.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;

use as2expert::AS2ExpertClient;
use eframe::egui::{self, Align, Align2, Color32, FontId, Layout, Rect, RichText, Stroke, Vec2};
use serde_json::{json, Value};

use crate::config::Config;
use crate::icons::{self, Icon};

// --- Palette (light, professional) -------------------------------------------
const ACCENT: Color32 = Color32::from_rgb(0x10, 0x6E, 0xBE);
const ACCENT_SOFT: Color32 = Color32::from_rgb(0xDB, 0xE8, 0xF7);
const HOVER_SOFT: Color32 = Color32::from_rgb(0xED, 0xF2, 0xF9);
const PANEL: Color32 = Color32::from_rgb(0xF3, 0xF4, 0xF6);
const CARD: Color32 = Color32::from_rgb(0xFF, 0xFF, 0xFF);
const BORDER: Color32 = Color32::from_rgb(0xE1, 0xE3, 0xE8);
const TEXT: Color32 = Color32::from_rgb(0x20, 0x24, 0x2A);
const MUTED: Color32 = Color32::from_rgb(0x6B, 0x72, 0x80);
const DANGER: Color32 = Color32::from_rgb(0xC4, 0x3B, 0x3B);
const OK_GREEN: Color32 = Color32::from_rgb(0x1E, 0x8E, 0x3E);

/// Results delivered from background API tasks to the UI thread.
enum Event {
    Stations(as2expert::Result<Vec<Value>>),
    Messages(as2expert::Result<Vec<Value>>),
    Opened(as2expert::Result<Value>),
    Body(as2expert::Result<Vec<u8>>),
    Partners(as2expert::Result<Vec<Value>>),
    Sent(as2expert::Result<Value>),
    Acted {
        label: String,
        res: as2expert::Result<Value>,
    },
}

#[derive(PartialEq)]
enum Screen {
    Login,
    Main,
}

pub struct App {
    rt: Arc<tokio::runtime::Runtime>,
    ctx: egui::Context,
    tx: Sender<Event>,
    rx: Receiver<Event>,

    config: Config,
    client: Option<AS2ExpertClient>,
    screen: Screen,

    // Data
    stations: Vec<Value>,
    selected_station: Option<i64>,
    messages: Vec<Value>,
    selected: Option<usize>,
    opened: Option<Value>,
    body_text: Option<String>,
    body_bytes: Option<Vec<u8>>,
    body_note: Option<String>,
    partners: Vec<Value>,

    // UI
    search: String,
    inflight: u32,
    error: Option<String>,
    status: String,

    // Compose
    compose_open: bool,
    compose_partner: usize,
    compose_subject: String,
    compose_file: String,
    compose_sending: bool,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        egui_extras::install_image_loaders(&cc.egui_ctx);
        apply_theme(&cc.egui_ctx);

        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("failed to start async runtime");
        let (tx, rx) = std::sync::mpsc::channel();
        Self {
            rt: Arc::new(rt),
            ctx: cc.egui_ctx.clone(),
            tx,
            rx,
            config: Config::load(),
            client: None,
            screen: Screen::Login,
            stations: Vec::new(),
            selected_station: None,
            messages: Vec::new(),
            selected: None,
            opened: None,
            body_text: None,
            body_bytes: None,
            body_note: None,
            partners: Vec::new(),
            search: String::new(),
            inflight: 0,
            error: None,
            status: String::new(),
            compose_open: false,
            compose_partner: 0,
            compose_subject: String::new(),
            compose_file: String::new(),
            compose_sending: false,
        }
    }

    // --- Background actions --------------------------------------------------

    fn connect(&mut self) {
        let base = self.config.resolved_base_url();
        if base.is_empty() {
            self.error = Some("Choose an environment or set a base URL.".into());
            return;
        }
        if self.config.token.trim().is_empty() {
            self.error = Some("An API token is required.".into());
            return;
        }
        match AS2ExpertClient::builder(self.config.token.trim())
            .base_url(base)
            .build()
        {
            Ok(c) => {
                self.client = Some(c);
                self.error = None;
                self.status = "Connecting…".into();
                self.load_stations();
                self.load_partners();
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    fn load_stations(&mut self) {
        let Some(c) = self.client.clone() else { return };
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        self.inflight += 1;
        self.rt.spawn(async move {
            let r = c.stations.list(json!({})).await;
            let _ = tx.send(Event::Stations(r));
            ctx.request_repaint();
        });
    }

    fn load_partners(&mut self) {
        let Some(c) = self.client.clone() else { return };
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        self.rt.spawn(async move {
            let r = c.partners.list(json!({})).await;
            let _ = tx.send(Event::Partners(r));
            ctx.request_repaint();
        });
    }

    fn refresh_messages(&mut self) {
        let Some(c) = self.client.clone() else { return };
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        let mut body = json!({ "limit": 200 });
        if let Some(sid) = self.selected_station {
            body["station"] = json!(sid);
        }
        self.inflight += 1;
        self.status = "Loading messages…".into();
        self.rt.spawn(async move {
            let r = c.messages.list(body).await;
            let _ = tx.send(Event::Messages(r));
            ctx.request_repaint();
        });
    }

    fn open_message(&mut self, index: usize) {
        self.selected = Some(index);
        self.opened = None;
        self.body_text = None;
        self.body_bytes = None;
        self.body_note = None;
        let Some(id) = self.messages.get(index).and_then(|m| m.get("id")).cloned() else {
            return;
        };
        let Some(c) = self.client.clone() else { return };
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        self.inflight += 1;
        let id2 = id.clone();
        let c2 = c.clone();
        let tx2 = tx.clone();
        let ctx2 = ctx.clone();
        self.rt.spawn(async move {
            let r = c.messages.get(id).await;
            let _ = tx.send(Event::Opened(r));
            ctx.request_repaint();
        });
        self.inflight += 1;
        self.rt.spawn(async move {
            let r = c2.messages.download(id2).await;
            let _ = tx2.send(Event::Body(r));
            ctx2.request_repaint();
        });
    }

    fn message_action(&mut self, path: &'static str, label: &str) {
        let Some(i) = self.selected else { return };
        let Some(id) = self.messages.get(i).and_then(|m| m.get("id")).cloned() else {
            return;
        };
        let Some(c) = self.client.clone() else { return };
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        let label = label.to_string();
        self.inflight += 1;
        self.status = format!("{label}…");
        self.rt.spawn(async move {
            let res = match path {
                "mark-read" => c.messages.mark_read(id).await,
                "mark-unread" => c.messages.mark_unread(id).await,
                "delete" => c.messages.delete(id).await,
                _ => c.messages.get(id).await,
            };
            let _ = tx.send(Event::Acted { label, res });
            ctx.request_repaint();
        });
    }

    fn send_message(&mut self) {
        let path = self.compose_file.trim().to_string();
        if path.is_empty() {
            self.error = Some("Pick a file to send.".into());
            return;
        }
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                self.error = Some(format!("Cannot read {path}: {e}"));
                return;
            }
        };
        let Some(partner) = self.partners.get(self.compose_partner) else {
            self.error = Some("Pick a partner.".into());
            return;
        };
        let partner_id = partner
            .get("id")
            .cloned()
            .unwrap_or_else(|| json!(sfield(partner, &["as2_id", "as2id"])));
        let file_name = Path::new(&path)
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "message.edi".into());
        let subject = if self.compose_subject.trim().is_empty() {
            file_name.clone()
        } else {
            self.compose_subject.trim().to_string()
        };
        let Some(c) = self.client.clone() else { return };
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        self.compose_sending = true;
        self.inflight += 1;
        self.rt.spawn(async move {
            let r = c
                .messages
                .send(partner_id, subject, file_name, &bytes)
                .await;
            let _ = tx.send(Event::Sent(r));
            ctx.request_repaint();
        });
    }

    fn save_payload(&mut self) {
        let Some(bytes) = self.body_bytes.clone() else {
            return;
        };
        let name = self
            .opened
            .as_ref()
            .map(|m| sfield(m, &["message_id", "idmensaje", "id"]))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "payload".into());
        let safe: String = name
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let dest = downloads_dir().join(format!("{safe}.edi"));
        match std::fs::write(&dest, &bytes) {
            Ok(_) => self.status = format!("Saved payload → {}", dest.display()),
            Err(e) => self.error = Some(format!("Save failed: {e}")),
        }
    }

    fn logout(&mut self) {
        self.client = None;
        self.screen = Screen::Login;
        self.messages.clear();
        self.stations.clear();
        self.partners.clear();
        self.selected = None;
        self.opened = None;
        self.body_text = None;
        self.body_bytes = None;
        self.status.clear();
    }

    // --- Event pump ----------------------------------------------------------

    fn drain_events(&mut self) {
        while let Ok(ev) = self.rx.try_recv() {
            match ev {
                Event::Stations(r) => {
                    self.inflight = self.inflight.saturating_sub(1);
                    match r {
                        Ok(v) => {
                            self.stations = v;
                            if self.screen == Screen::Login {
                                self.screen = Screen::Main;
                                let _ = self.config.save();
                                self.status = "Connected.".into();
                                self.refresh_messages();
                            }
                        }
                        Err(e) => {
                            self.error = Some(e.to_string());
                            if self.screen == Screen::Login {
                                self.client = None;
                            }
                        }
                    }
                }
                Event::Messages(r) => {
                    self.inflight = self.inflight.saturating_sub(1);
                    match r {
                        Ok(v) => {
                            self.status = format!("{} messages", v.len());
                            self.messages = v;
                            self.selected = None;
                            self.opened = None;
                            self.body_text = None;
                        }
                        Err(e) => self.error = Some(e.to_string()),
                    }
                }
                Event::Opened(r) => {
                    self.inflight = self.inflight.saturating_sub(1);
                    match r {
                        Ok(v) => self.opened = Some(v),
                        Err(e) => self.error = Some(e.to_string()),
                    }
                }
                Event::Body(r) => {
                    self.inflight = self.inflight.saturating_sub(1);
                    match r {
                        Ok(bytes) => {
                            match String::from_utf8(bytes.clone()) {
                                Ok(t) => {
                                    self.body_note = Some(format!("{} bytes", bytes.len()));
                                    self.body_text = Some(t);
                                }
                                Err(_) => {
                                    self.body_text = None;
                                    self.body_note =
                                        Some(format!("binary payload, {} bytes", bytes.len()));
                                }
                            }
                            self.body_bytes = Some(bytes);
                        }
                        Err(e) => self.body_note = Some(format!("download failed: {e}")),
                    }
                }
                Event::Partners(r) => {
                    if let Ok(v) = r {
                        self.partners = v;
                    }
                }
                Event::Sent(r) => {
                    self.inflight = self.inflight.saturating_sub(1);
                    self.compose_sending = false;
                    match r {
                        Ok(_) => {
                            self.compose_open = false;
                            self.compose_subject.clear();
                            self.compose_file.clear();
                            self.status = "Message sent.".into();
                            self.refresh_messages();
                        }
                        Err(e) => self.error = Some(format!("Send failed: {e}")),
                    }
                }
                Event::Acted { label, res } => {
                    self.inflight = self.inflight.saturating_sub(1);
                    match res {
                        Ok(_) => {
                            self.status = format!("{label} ✓");
                            self.refresh_messages();
                        }
                        Err(e) => self.error = Some(format!("{label} failed: {e}")),
                    }
                }
            }
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_events();
        ctx.input(|i| {
            if let Some(f) = i.raw.dropped_files.first() {
                if let Some(p) = &f.path {
                    self.compose_file = p.display().to_string();
                    self.compose_open = true;
                }
            }
        });

        match self.screen {
            Screen::Login => self.ui_login(ctx),
            Screen::Main => self.ui_main(ctx),
        }
    }
}

// --- UI ----------------------------------------------------------------------

impl App {
    fn ui_login(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(egui::Frame::default().fill(PANEL))
            .show(ctx, |ui| {
                ui.add_space(56.0);
                ui.vertical_centered(|ui| {
                    icons::show(ui, Icon::Logo, 56.0);
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new("AS2Expert Desktop")
                            .size(24.0)
                            .strong()
                            .color(TEXT),
                    );
                    ui.label(RichText::new("Connect to your AS2Expert account").color(MUTED));
                    ui.add_space(18.0);
                });

                ui.vertical_centered(|ui| {
                    egui::Frame::default()
                        .fill(CARD)
                        .stroke(Stroke::new(1.0, BORDER))
                        .rounding(10.0)
                        .inner_margin(20.0)
                        .show(ui, |ui| {
                            ui.set_width(440.0);
                            egui::Grid::new("login")
                                .num_columns(2)
                                .spacing([12.0, 12.0])
                                .show(ui, |ui| {
                                    ui.label(RichText::new("Environment").color(MUTED));
                                    egui::ComboBox::from_id_salt("env")
                                        .width(260.0)
                                        .selected_text(pretty_env(&self.config.environment))
                                        .show_ui(ui, |ui| {
                                            ui.selectable_value(
                                                &mut self.config.environment,
                                                "free".into(),
                                                "Free — free.as2expert.com",
                                            );
                                            ui.selectable_value(
                                                &mut self.config.environment,
                                                "b2b".into(),
                                                "B2B — b2b.as2expert.com",
                                            );
                                            ui.selectable_value(
                                                &mut self.config.environment,
                                                "custom".into(),
                                                "Custom base URL",
                                            );
                                        });
                                    ui.end_row();

                                    if self.config.environment == "custom" {
                                        ui.label(RichText::new("Base URL").color(MUTED));
                                        ui.add(
                                            egui::TextEdit::singleline(&mut self.config.base_url)
                                                .hint_text("https://your-host/api/v1")
                                                .desired_width(260.0),
                                        );
                                        ui.end_row();
                                    }

                                    ui.label(RichText::new("API token").color(MUTED));
                                    ui.add(
                                        egui::TextEdit::singleline(&mut self.config.token)
                                            .password(true)
                                            .hint_text("Bearer token")
                                            .desired_width(260.0),
                                    );
                                    ui.end_row();

                                    ui.label("");
                                    ui.checkbox(
                                        &mut self.config.remember_token,
                                        "Remember token on this device",
                                    );
                                    ui.end_row();
                                });

                            ui.add_space(14.0);
                            let busy = self.inflight > 0;
                            ui.horizontal(|ui| {
                                let connect = ui.add_enabled(
                                    !busy,
                                    egui::Button::image_and_text(
                                        icons::image(Icon::Key, 18.0),
                                        RichText::new("  Connect  ").strong(),
                                    )
                                    .fill(ACCENT),
                                );
                                if connect.clicked() {
                                    self.connect();
                                }
                                if busy {
                                    ui.add_space(6.0);
                                    ui.spinner();
                                }
                            });

                            if let Some(err) = self.error.clone() {
                                ui.add_space(10.0);
                                ui.colored_label(DANGER, err);
                            }
                        });
                });
            });
    }

    fn ui_main(&mut self, ctx: &egui::Context) {
        self.ui_toolbar(ctx);
        self.ui_statusbar(ctx);
        self.ui_stations(ctx);
        if self.selected.is_some() {
            self.ui_detail(ctx);
        }
        self.ui_list(ctx);
        if self.compose_open {
            self.ui_compose(ctx);
        }
    }

    fn ui_toolbar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("toolbar")
            .frame(
                egui::Frame::default()
                    .fill(CARD)
                    .stroke(Stroke::new(1.0, BORDER))
                    .inner_margin(egui::Margin::symmetric(10.0, 7.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if icons::labeled_button(ui, Icon::Refresh, 18.0, "Receive").clicked() {
                        self.refresh_messages();
                    }
                    if icons::labeled_button(ui, Icon::Compose, 18.0, "New message").clicked() {
                        if self.partners.is_empty() {
                            self.load_partners();
                        }
                        self.compose_open = true;
                    }
                    ui.separator();
                    icons::show(ui, Icon::Search, 16.0);
                    ui.add(
                        egui::TextEdit::singleline(&mut self.search)
                            .hint_text("Search subject or partner")
                            .desired_width(240.0),
                    );
                    if !self.search.is_empty() && ui.small_button("✕").clicked() {
                        self.search.clear();
                    }

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if icons::tool_button(ui, Icon::Key, 18.0, "Log out").clicked() {
                            self.logout();
                        }
                        if self.inflight > 0 {
                            ui.spinner();
                        }
                    });
                });
            });

        if let Some(err) = self.error.clone() {
            egui::TopBottomPanel::top("errbar")
                .frame(
                    egui::Frame::default()
                        .fill(Color32::from_rgb(0xFD, 0xEC, 0xEC))
                        .inner_margin(egui::Margin::symmetric(10.0, 6.0)),
                )
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        icons::show(ui, Icon::Warning, 16.0);
                        ui.colored_label(DANGER, err);
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui.small_button("Dismiss").clicked() {
                                self.error = None;
                            }
                        });
                    });
                });
        }
    }

    fn ui_statusbar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("statusbar")
            .frame(
                egui::Frame::default()
                    .fill(PANEL)
                    .stroke(Stroke::new(1.0, BORDER))
                    .inner_margin(egui::Margin::symmetric(10.0, 4.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    icons::show(ui, Icon::Station, 14.0);
                    ui.label(
                        RichText::new(self.config.resolved_base_url())
                            .small()
                            .color(MUTED),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(RichText::new(&self.status).small().color(MUTED));
                    });
                });
            });
    }

    fn ui_stations(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("stations")
            .resizable(true)
            .default_width(232.0)
            .frame(
                egui::Frame::default()
                    .fill(PANEL)
                    .inner_margin(egui::Margin::symmetric(8.0, 8.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    icons::show(ui, Icon::Inbox, 18.0);
                    ui.label(RichText::new("Stations").strong().color(TEXT));
                });
                ui.add_space(6.0);

                let mut changed = None;
                if station_row(
                    ui,
                    Icon::Inbox,
                    "All stations",
                    "",
                    self.selected_station.is_none(),
                )
                .clicked()
                {
                    changed = Some(None);
                }
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for st in &self.stations {
                        let id = st.get("id").and_then(|v| v.as_i64());
                        let name = sfield(st, &["name", "nombre"]);
                        let as2 = sfield(st, &["as2_id", "as2id"]);
                        let sel = self.selected_station == id && id.is_some();
                        if station_row(ui, Icon::Station, &name, &as2, sel).clicked() {
                            changed = Some(id);
                        }
                    }
                });
                if let Some(sel) = changed {
                    self.selected_station = sel;
                    self.refresh_messages();
                }
            });
    }

    fn ui_list(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(egui::Frame::default().fill(CARD).inner_margin(0.0))
            .show(ctx, |ui| {
                let needle = self.search.to_lowercase();
                let mut to_open = None;
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let mut shown = 0;
                        for (i, m) in self.messages.iter().enumerate() {
                            let subject = sfield(m, &["subject", "asunto"]);
                            let partner = sfield(m, &["partner_name", "socio_nombre"]);
                            if !needle.is_empty()
                                && !subject.to_lowercase().contains(&needle)
                                && !partner.to_lowercase().contains(&needle)
                            {
                                continue;
                            }
                            shown += 1;
                            if message_row(ui, m, self.selected == Some(i)).clicked() {
                                to_open = Some(i);
                            }
                        }
                        if shown == 0 && self.inflight == 0 {
                            ui.add_space(40.0);
                            ui.vertical_centered(|ui| {
                                icons::show(ui, Icon::Inbox, 40.0);
                                ui.label(RichText::new("No messages").color(MUTED));
                                ui.label(
                                    RichText::new("Use Receive, or pick another station.")
                                        .small()
                                        .color(MUTED),
                                );
                            });
                        }
                    });
                if let Some(i) = to_open {
                    self.open_message(i);
                }
            });
    }

    fn ui_detail(&mut self, ctx: &egui::Context) {
        egui::SidePanel::right("detail")
            .resizable(true)
            .default_width(480.0)
            .frame(
                egui::Frame::default()
                    .fill(CARD)
                    .stroke(Stroke::new(1.0, BORDER))
                    .inner_margin(14.0),
            )
            .show(ctx, |ui| {
                let m = match self.opened.clone() {
                    Some(m) => m,
                    None => {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label("Loading…");
                        });
                        return;
                    }
                };

                let incoming = m
                    .get("incoming")
                    .or_else(|| m.get("entrante"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                ui.horizontal(|ui| {
                    icons::show(ui, if incoming { Icon::In } else { Icon::Out }, 26.0);
                    ui.label(
                        RichText::new(sfield(&m, &["subject", "asunto"]))
                            .size(16.0)
                            .strong()
                            .color(TEXT),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.small_button("Close").clicked() {
                            self.selected = None;
                            self.opened = None;
                        }
                    });
                });
                ui.separator();

                egui::Grid::new("detail_grid")
                    .num_columns(3)
                    .spacing([8.0, 7.0])
                    .show(ui, |ui| {
                        meta_row(
                            ui,
                            Icon::Partner,
                            "Partner",
                            &sfield(&m, &["partner_name", "socio_nombre"]),
                        );
                        meta_row(
                            ui,
                            Icon::Station,
                            "Station",
                            &sfield(&m, &["station_name", "estacion_nombre"]),
                        );
                        meta_row(ui, Icon::Message, "Date", &sfield(&m, &["date", "fecha"]));
                        meta_row(
                            ui,
                            Icon::Certificate,
                            "AS2 ID",
                            &sfield(&m, &["partner_as2_id", "socio_as2id"]),
                        );
                        meta_row(ui, Icon::Ok, "MDN", &sfield(&m, &["mdn"]));
                        meta_row(ui, Icon::Lock, "Encryption", &sfield(&m, &["encriptacion"]));
                        meta_row(ui, Icon::Key, "Signature", &sfield(&m, &["firma"]));
                    });

                ui.add_space(10.0);
                ui.horizontal_wrapped(|ui| {
                    if icons::labeled_button(ui, Icon::Read, 16.0, "Mark read").clicked() {
                        self.message_action("mark-read", "Mark read");
                    }
                    if icons::labeled_button(ui, Icon::Unread, 16.0, "Mark unread").clicked() {
                        self.message_action("mark-unread", "Mark unread");
                    }
                    if icons::labeled_button(ui, Icon::Save, 16.0, "Save payload").clicked() {
                        self.save_payload();
                    }
                    if icons::labeled_button(ui, Icon::Delete, 16.0, "Delete").clicked() {
                        self.message_action("delete", "Delete");
                        self.selected = None;
                        self.opened = None;
                    }
                });

                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    icons::show(ui, Icon::Attach, 16.0);
                    ui.label(RichText::new("Payload").strong().color(TEXT));
                    if let Some(note) = &self.body_note {
                        ui.label(RichText::new(note).small().color(MUTED));
                    }
                });
                ui.separator();
                egui::ScrollArea::both()
                    .auto_shrink([false, false])
                    .show(ui, |ui| match &self.body_text {
                        Some(t) => {
                            ui.add(
                                egui::TextEdit::multiline(&mut t.as_str())
                                    .font(egui::TextStyle::Monospace)
                                    .desired_width(f32::INFINITY)
                                    .desired_rows(16),
                            );
                        }
                        None => {
                            ui.label(RichText::new("(no text payload)").color(MUTED));
                        }
                    });
            });
    }

    fn ui_compose(&mut self, ctx: &egui::Context) {
        let mut open = self.compose_open;
        egui::Window::new(RichText::new("  New message").strong())
            .open(&mut open)
            .resizable(true)
            .collapsible(false)
            .default_width(480.0)
            .show(ctx, |ui| {
                egui::Grid::new("compose")
                    .num_columns(2)
                    .spacing([10.0, 10.0])
                    .show(ui, |ui| {
                        label_with_icon(ui, Icon::Partner, "Partner");
                        let current = self
                            .partners
                            .get(self.compose_partner)
                            .map(partner_label)
                            .unwrap_or_else(|| "— pick a partner —".into());
                        egui::ComboBox::from_id_salt("partner")
                            .selected_text(current)
                            .width(320.0)
                            .show_ui(ui, |ui| {
                                for (i, p) in self.partners.iter().enumerate() {
                                    ui.selectable_value(
                                        &mut self.compose_partner,
                                        i,
                                        partner_label(p),
                                    );
                                }
                            });
                        ui.end_row();

                        label_with_icon(ui, Icon::Message, "Subject");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.compose_subject)
                                .hint_text("(defaults to the file name)")
                                .desired_width(320.0),
                        );
                        ui.end_row();

                        label_with_icon(ui, Icon::Attach, "File");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.compose_file)
                                .hint_text("path to the EDI file — or drag one onto the window")
                                .desired_width(320.0),
                        );
                        ui.end_row();
                    });

                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    let busy = self.compose_sending;
                    let ready = !self.partners.is_empty() && !self.compose_file.trim().is_empty();
                    let send = ui.add_enabled(
                        !busy && ready,
                        egui::Button::image_and_text(
                            icons::image(Icon::Send, 18.0),
                            RichText::new("Send").strong(),
                        )
                        .fill(ACCENT),
                    );
                    if send.clicked() {
                        self.send_message();
                    }
                    if busy {
                        ui.spinner();
                    }
                    if self.partners.is_empty() {
                        ui.label(RichText::new("Loading partners…").small().color(MUTED));
                    }
                });
            });
        self.compose_open = open;
    }
}

// --- Theme -------------------------------------------------------------------

fn apply_theme(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    let mut v = egui::Visuals::light();
    v.panel_fill = PANEL;
    v.window_fill = CARD;
    v.extreme_bg_color = CARD;
    v.override_text_color = Some(TEXT);
    v.hyperlink_color = ACCENT;
    v.selection.bg_fill = ACCENT_SOFT;
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    let rounding = egui::Rounding::same(6.0);
    v.widgets.noninteractive.rounding = rounding;
    v.widgets.inactive.rounding = rounding;
    v.widgets.hovered.rounding = rounding;
    v.widgets.active.rounding = rounding;
    v.widgets.open.rounding = rounding;
    v.widgets.hovered.weak_bg_fill = HOVER_SOFT;
    v.widgets.inactive.weak_bg_fill = Color32::from_rgb(0xEC, 0xEE, 0xF1);
    style.visuals = v;

    style.spacing.item_spacing = Vec2::new(8.0, 6.0);
    style.spacing.button_padding = Vec2::new(10.0, 6.0);
    style.spacing.window_margin = egui::Margin::same(12.0);
    style.spacing.interact_size.y = 26.0;

    use egui::FontFamily::{Monospace, Proportional};
    use egui::TextStyle::{Body, Button, Heading, Monospace as Mono, Small};
    style.text_styles = [
        (Heading, FontId::new(20.0, Proportional)),
        (Body, FontId::new(14.0, Proportional)),
        (Button, FontId::new(14.0, Proportional)),
        (Small, FontId::new(11.5, Proportional)),
        (Mono, FontId::new(12.5, Monospace)),
    ]
    .into();

    ctx.set_style(style);
}

// --- Row widgets -------------------------------------------------------------

/// A station/folder entry in the left sidebar. Returns its click response.
fn station_row(
    ui: &mut egui::Ui,
    icon: Icon,
    name: &str,
    sub: &str,
    selected: bool,
) -> egui::Response {
    let two_line = !sub.is_empty();
    let height = if two_line { 42.0 } else { 30.0 };
    let (rect, resp) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), height),
        egui::Sense::click(),
    );
    let bg = if selected {
        ACCENT_SOFT
    } else if resp.hovered() {
        HOVER_SOFT
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, 6.0, bg);
    let icon_rect = Rect::from_min_size(
        egui::pos2(rect.left() + 8.0, rect.center().y - 9.0),
        Vec2::splat(18.0),
    );
    icons::image(icon, 18.0).paint_at(ui, icon_rect);
    let tx = icon_rect.right() + 8.0;
    let p = ui.painter();
    let name_color = if selected { ACCENT } else { TEXT };
    if two_line {
        p.text(
            egui::pos2(tx, rect.top() + 6.0),
            Align2::LEFT_TOP,
            trunc(name, 26),
            FontId::proportional(13.0),
            name_color,
        );
        p.text(
            egui::pos2(tx, rect.top() + 23.0),
            Align2::LEFT_TOP,
            trunc(sub, 28),
            FontId::proportional(11.0),
            MUTED,
        );
    } else {
        p.text(
            egui::pos2(tx, rect.center().y),
            Align2::LEFT_CENTER,
            name,
            FontId::proportional(13.5),
            name_color,
        );
    }
    resp
}

/// An Outlook-style message row. Returns its click response.
fn message_row(ui: &mut egui::Ui, m: &Value, selected: bool) -> egui::Response {
    let height = 50.0;
    let (rect, resp) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), height),
        egui::Sense::click(),
    );
    let bg = if selected {
        ACCENT_SOFT
    } else if resp.hovered() {
        HOVER_SOFT
    } else {
        CARD
    };
    ui.painter().rect_filled(rect, 0.0, bg);
    // left accent bar when selected
    if selected {
        ui.painter().rect_filled(
            Rect::from_min_size(rect.left_top(), Vec2::new(3.0, rect.height())),
            0.0,
            ACCENT,
        );
    }
    ui.painter().hline(
        rect.x_range(),
        rect.bottom(),
        Stroke::new(1.0, Color32::from_rgb(0xEF, 0xF0, 0xF2)),
    );

    let incoming = m
        .get("incoming")
        .or_else(|| m.get("entrante"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let icon_rect = Rect::from_min_size(
        egui::pos2(rect.left() + 12.0, rect.center().y - 12.0),
        Vec2::splat(24.0),
    );
    icons::image(if incoming { Icon::In } else { Icon::Out }, 24.0).paint_at(ui, icon_rect);

    let subject = sfield(m, &["subject", "asunto"]);
    let partner = sfield(m, &["partner_name", "socio_nombre"]);
    let folder = sfield(m, &["folder_name"]);
    let mdn = sfield(m, &["mdn"]);
    let date = short_date(&sfield(m, &["date", "fecha"]));

    let tx = icon_rect.right() + 10.0;
    let p = ui.painter();
    let subj = if subject.is_empty() {
        "(no subject)".to_string()
    } else {
        subject
    };
    p.text(
        egui::pos2(tx, rect.top() + 9.0),
        Align2::LEFT_TOP,
        trunc(&subj, 52),
        FontId::proportional(14.0),
        TEXT,
    );
    let meta = format!("{}  ·  {}", trunc(&partner, 34), folder);
    p.text(
        egui::pos2(tx, rect.top() + 28.0),
        Align2::LEFT_TOP,
        meta,
        FontId::proportional(11.5),
        MUTED,
    );

    // right column: date + MDN status
    p.text(
        egui::pos2(rect.right() - 12.0, rect.top() + 9.0),
        Align2::RIGHT_TOP,
        date,
        FontId::proportional(11.5),
        MUTED,
    );
    if !mdn.is_empty() {
        let ok = mdn.eq_ignore_ascii_case("ok");
        p.text(
            egui::pos2(rect.right() - 12.0, rect.top() + 27.0),
            Align2::RIGHT_TOP,
            format!("MDN {mdn}"),
            FontId::proportional(11.0),
            if ok { OK_GREEN } else { DANGER },
        );
    }
    resp
}

/// A metadata row in the reading pane: small icon, muted label, value.
fn meta_row(ui: &mut egui::Ui, icon: Icon, label: &str, value: &str) {
    if value.is_empty() {
        return;
    }
    icons::show(ui, icon, 15.0);
    ui.label(RichText::new(label).color(MUTED));
    ui.label(RichText::new(value).color(TEXT));
    ui.end_row();
}

fn label_with_icon(ui: &mut egui::Ui, icon: Icon, text: &str) {
    ui.horizontal(|ui| {
        icons::show(ui, icon, 16.0);
        ui.label(RichText::new(text).color(MUTED));
    });
}

// --- helpers -----------------------------------------------------------------

/// Return the first present, non-empty string among `keys`.
fn sfield(v: &Value, keys: &[&str]) -> String {
    for k in keys {
        match v.get(*k) {
            Some(Value::String(s)) if !s.is_empty() => return s.clone(),
            Some(Value::Number(n)) => return n.to_string(),
            Some(Value::Bool(b)) => return b.to_string(),
            _ => {}
        }
    }
    String::new()
}

fn partner_label(p: &Value) -> String {
    let name = sfield(p, &["name", "nombre"]);
    let as2 = sfield(p, &["as2_id", "as2id"]);
    if as2.is_empty() {
        name
    } else {
        format!("{name} ({as2})")
    }
}

fn pretty_env(env: &str) -> &str {
    match env {
        "b2b" => "B2B — b2b.as2expert.com",
        "custom" => "Custom base URL",
        _ => "Free — free.as2expert.com",
    }
}

fn short_date(d: &str) -> String {
    let cut = d.split('.').next().unwrap_or(d);
    if cut.len() >= 16 {
        cut[..16].to_string()
    } else {
        cut.to_string()
    }
}

fn trunc(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(n.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

fn downloads_dir() -> PathBuf {
    if let Some(home) = std::env::var_os("HOME") {
        let d = PathBuf::from(&home).join("Downloads");
        if d.is_dir() {
            return d;
        }
        return PathBuf::from(home);
    }
    if let Some(up) = std::env::var_os("USERPROFILE") {
        return PathBuf::from(up).join("Downloads");
    }
    std::env::temp_dir()
}
