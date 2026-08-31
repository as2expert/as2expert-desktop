//! The AS2Expert desktop application: an email-client-style UI over the SDK.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;

use as2expert::AS2ExpertClient;
use eframe::egui;
use serde_json::{json, Value};

use crate::config::Config;

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
                            self.status = format!("{} messages.", v.len());
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
        // Absorb any file dropped onto the window into the compose field.
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
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(60.0);
                ui.heading("AS2Expert Desktop");
                ui.label("Connect to your AS2Expert account.");
                ui.add_space(20.0);
            });
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.set_max_width(520.0);
                egui::Grid::new("login")
                    .num_columns(2)
                    .spacing([12.0, 10.0])
                    .show(ui, |ui| {
                        ui.label("Environment");
                        egui::ComboBox::from_id_salt("env")
                            .selected_text(pretty_env(&self.config.environment))
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut self.config.environment,
                                    "free".into(),
                                    "Free (free.as2expert.com)",
                                );
                                ui.selectable_value(
                                    &mut self.config.environment,
                                    "b2b".into(),
                                    "B2B (b2b.as2expert.com)",
                                );
                                ui.selectable_value(
                                    &mut self.config.environment,
                                    "custom".into(),
                                    "Custom base URL",
                                );
                            });
                        ui.end_row();

                        if self.config.environment == "custom" {
                            ui.label("Base URL");
                            ui.add(
                                egui::TextEdit::singleline(&mut self.config.base_url)
                                    .hint_text("https://your-host/api/v1")
                                    .desired_width(320.0),
                            );
                            ui.end_row();
                        }

                        ui.label("API token");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.config.token)
                                .password(true)
                                .hint_text("Bearer token")
                                .desired_width(320.0),
                        );
                        ui.end_row();

                        ui.label("");
                        ui.checkbox(
                            &mut self.config.remember_token,
                            "Remember token on this device",
                        );
                        ui.end_row();
                    });

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let busy = self.inflight > 0;
                    if ui
                        .add_enabled(!busy, egui::Button::new("Connect"))
                        .clicked()
                    {
                        self.connect();
                    }
                    if busy {
                        ui.spinner();
                    }
                });
            });

            if let Some(err) = self.error.clone() {
                ui.add_space(10.0);
                ui.colored_label(egui::Color32::from_rgb(0xd3, 0x3a, 0x3a), err);
            }
        });
    }

    fn ui_main(&mut self, ctx: &egui::Context) {
        self.ui_toolbar(ctx);
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
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui.button("⟳ Receive / Refresh").clicked() {
                    self.refresh_messages();
                }
                if ui.button("✉ New message").clicked() {
                    if self.partners.is_empty() {
                        self.load_partners();
                    }
                    self.compose_open = true;
                }
                ui.separator();
                ui.label("Search:");
                ui.add(
                    egui::TextEdit::singleline(&mut self.search)
                        .hint_text("subject or partner")
                        .desired_width(220.0),
                );
                if !self.search.is_empty() && ui.button("✕").clicked() {
                    self.search.clear();
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Log out").clicked() {
                        self.logout();
                    }
                    if self.inflight > 0 {
                        ui.spinner();
                    }
                    ui.label(&self.status);
                });
            });
            ui.add_space(4.0);
        });
        if let Some(err) = self.error.clone() {
            egui::TopBottomPanel::top("errbar").show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.colored_label(
                        egui::Color32::from_rgb(0xd3, 0x3a, 0x3a),
                        format!("⚠ {err}"),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Dismiss").clicked() {
                            self.error = None;
                        }
                    });
                });
            });
        }
    }

    fn ui_stations(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("stations")
            .resizable(true)
            .default_width(220.0)
            .show(ctx, |ui| {
                ui.add_space(6.0);
                ui.heading("Stations");
                ui.separator();
                let mut changed = None;
                if ui
                    .selectable_label(self.selected_station.is_none(), "📥 All stations")
                    .clicked()
                {
                    changed = Some(None);
                }
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for st in &self.stations {
                        let id = st.get("id").and_then(|v| v.as_i64());
                        let name = sfield(st, &["name", "nombre"]);
                        let as2 = sfield(st, &["as2_id", "as2id"]);
                        let label = if as2.is_empty() {
                            name.clone()
                        } else {
                            format!("{name}\n{as2}")
                        };
                        if ui
                            .selectable_label(self.selected_station == id && id.is_some(), label)
                            .clicked()
                        {
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
        egui::CentralPanel::default().show(ctx, |ui| {
            let needle = self.search.to_lowercase();
            let mut to_open = None;
            egui::ScrollArea::vertical().show(ui, |ui| {
                for (i, m) in self.messages.iter().enumerate() {
                    let subject = sfield(m, &["subject", "asunto"]);
                    let partner = sfield(m, &["partner_name", "socio_nombre"]);
                    let date = sfield(m, &["date", "fecha"]);
                    let folder = sfield(m, &["folder_name"]);
                    let mdn = sfield(m, &["mdn"]);
                    let incoming = m
                        .get("incoming")
                        .or_else(|| m.get("entrante"))
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    if !needle.is_empty()
                        && !subject.to_lowercase().contains(&needle)
                        && !partner.to_lowercase().contains(&needle)
                    {
                        continue;
                    }
                    let dir = if incoming { "⬇" } else { "⬆" };
                    let head = format!(
                        "{dir}  {}",
                        if subject.is_empty() {
                            "(no subject)"
                        } else {
                            &subject
                        }
                    );
                    let sub = format!(
                        "{partner}  ·  {}  ·  {folder}  ·  MDN {mdn}",
                        short_date(&date)
                    );
                    let text = egui::RichText::new(format!("{head}\n{sub}"));
                    if ui
                        .selectable_label(self.selected == Some(i), text)
                        .clicked()
                    {
                        to_open = Some(i);
                    }
                    ui.separator();
                }
                if self.messages.is_empty() && self.inflight == 0 {
                    ui.add_space(20.0);
                    ui.weak("No messages. Use Receive / Refresh, or pick a station.");
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
            .default_width(460.0)
            .show(ctx, |ui| {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.heading("Message");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Close").clicked() {
                            self.selected = None;
                            self.opened = None;
                        }
                    });
                });
                ui.separator();

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

                egui::Grid::new("detail_grid")
                    .num_columns(2)
                    .spacing([10.0, 6.0])
                    .show(ui, |ui| {
                        detail_row(ui, "Subject", &sfield(&m, &["subject", "asunto"]));
                        detail_row(
                            ui,
                            "Partner",
                            &sfield(&m, &["partner_name", "socio_nombre"]),
                        );
                        detail_row(
                            ui,
                            "AS2 ID",
                            &sfield(&m, &["partner_as2_id", "socio_as2id"]),
                        );
                        detail_row(
                            ui,
                            "Station",
                            &sfield(&m, &["station_name", "estacion_nombre"]),
                        );
                        detail_row(ui, "Date", &sfield(&m, &["date", "fecha"]));
                        detail_row(ui, "Message ID", &sfield(&m, &["message_id", "idmensaje"]));
                        detail_row(ui, "MDN", &sfield(&m, &["mdn"]));
                        detail_row(ui, "Encryption", &sfield(&m, &["encriptacion"]));
                        detail_row(ui, "Signature", &sfield(&m, &["firma"]));
                    });

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Mark read").clicked() {
                        self.message_action("mark-read", "Mark read");
                    }
                    if ui.button("Mark unread").clicked() {
                        self.message_action("mark-unread", "Mark unread");
                    }
                    if ui.button("Save payload").clicked() {
                        self.save_payload();
                    }
                    if ui.button("🗑 Delete").clicked() {
                        self.message_action("delete", "Delete");
                        self.selected = None;
                        self.opened = None;
                    }
                });

                ui.add_space(8.0);
                ui.label(egui::RichText::new("Payload").strong());
                if let Some(note) = &self.body_note {
                    ui.weak(note);
                }
                ui.separator();
                egui::ScrollArea::both()
                    .auto_shrink([false, false])
                    .show(ui, |ui| match &self.body_text {
                        Some(t) => {
                            ui.add(
                                egui::TextEdit::multiline(&mut t.as_str())
                                    .font(egui::TextStyle::Monospace)
                                    .desired_width(f32::INFINITY)
                                    .desired_rows(18),
                            );
                        }
                        None => {
                            ui.weak("(no text payload)");
                        }
                    });
            });
    }

    fn ui_compose(&mut self, ctx: &egui::Context) {
        let mut open = self.compose_open;
        egui::Window::new("New message")
            .open(&mut open)
            .resizable(true)
            .default_width(460.0)
            .show(ctx, |ui| {
                egui::Grid::new("compose")
                    .num_columns(2)
                    .spacing([10.0, 8.0])
                    .show(ui, |ui| {
                        ui.label("Partner");
                        let current = self
                            .partners
                            .get(self.compose_partner)
                            .map(partner_label)
                            .unwrap_or_else(|| "— pick a partner —".into());
                        egui::ComboBox::from_id_salt("partner")
                            .selected_text(current)
                            .width(300.0)
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

                        ui.label("Subject");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.compose_subject)
                                .hint_text("(defaults to the file name)")
                                .desired_width(300.0),
                        );
                        ui.end_row();

                        ui.label("File");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.compose_file)
                                .hint_text("path to the EDI file — or drag one onto the window")
                                .desired_width(300.0),
                        );
                        ui.end_row();
                    });

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let busy = self.compose_sending;
                    let ready = !self.partners.is_empty() && !self.compose_file.trim().is_empty();
                    if ui
                        .add_enabled(!busy && ready, egui::Button::new("Send"))
                        .clicked()
                    {
                        self.send_message();
                    }
                    if busy {
                        ui.spinner();
                    }
                    if self.partners.is_empty() {
                        ui.weak("Loading partners…");
                    }
                });
            });
        // Respect the window's own close button.
        self.compose_open = open;
    }
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

fn detail_row(ui: &mut egui::Ui, label: &str, value: &str) {
    if value.is_empty() {
        return;
    }
    ui.label(egui::RichText::new(label).weak());
    ui.label(value);
    ui.end_row();
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
        "b2b" => "B2B (b2b.as2expert.com)",
        "custom" => "Custom base URL",
        _ => "Free (free.as2expert.com)",
    }
}

fn short_date(d: &str) -> String {
    // "2026-08-30 10:13:17.192968" -> "2026-08-30 10:13"
    let cut = d.split('.').next().unwrap_or(d);
    if cut.len() >= 16 {
        cut[..16].to_string()
    } else {
        cut.to_string()
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
