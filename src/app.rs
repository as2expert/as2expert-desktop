//! The AS2Expert desktop application: a web-app-style client over the SDK.
//!
//! Layout mirrors the web portal: a station picker in the toolbar, a folder tree
//! on the left, a message grid in the centre, and a reading pane on the right.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;

use as2expert::AS2ExpertClient;
use eframe::egui::{self, Align, Align2, Color32, FontId, Layout, Rect, RichText, Stroke, Vec2};
use egui_extras::{Column, TableBuilder};
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

/// A folder in a station (from the `/messages/folders` endpoint, or derived from
/// the loaded messages as a fallback).
struct Folder {
    id: i64,
    name: String,
    parent: Option<i64>,
    count: usize,
}

#[derive(Clone, Copy, PartialEq)]
enum SortKey {
    Subject,
    Partner,
    Date,
    Mdn,
}

/// Results delivered from background API tasks to the UI thread.
enum Event {
    Stations(as2expert::Result<Vec<Value>>),
    Folders(as2expert::Result<Vec<Value>>),
    Messages(as2expert::Result<Vec<Value>>),
    Opened(as2expert::Result<Value>),
    Body(as2expert::Result<Vec<u8>>),
    Partners(as2expert::Result<Vec<Value>>),
    Certificates(as2expert::Result<Vec<Value>>),
    MaintDetail(as2expert::Result<Value>),
    Created {
        label: String,
        res: as2expert::Result<Value>,
    },
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

/// Top-level modules, switched from the left nav rail.
#[derive(Clone, Copy, PartialEq)]
enum View {
    Mail,
    Stations,
    Partners,
    Certificates,
}

/// Which "New …" form is open.
#[derive(Clone, Copy, PartialEq)]
enum NewKind {
    Station,
    Partner,
    Certificate,
}

/// Backing state for the create forms.
struct Forms {
    open: Option<NewKind>,
    /// Set when the form is editing an existing record (its id); None = create.
    edit_id: Option<Value>,
    busy: bool,
    // Station
    st_name: String,
    st_as2: String,
    st_email: String,
    // Partner
    pt_station: usize,
    pt_name: String,
    pt_as2: String,
    pt_email: String,
    pt_url: String,
    // Certificate (self-signed)
    ct_station: usize,
    ct_cn: String,
    ct_email: String,
    ct_country: String,
    ct_locality: String,
    ct_province: String,
    ct_org: String,
    ct_orgunit: String,
    ct_days: String,
    ct_keybits: String,
    ct_hash: String,
}

impl Default for Forms {
    fn default() -> Self {
        Forms {
            open: None,
            edit_id: None,
            busy: false,
            st_name: String::new(),
            st_as2: String::new(),
            st_email: String::new(),
            pt_station: 0,
            pt_name: String::new(),
            pt_as2: String::new(),
            pt_email: String::new(),
            pt_url: String::new(),
            ct_station: 0,
            ct_cn: String::new(),
            ct_email: String::new(),
            ct_country: "ES".into(),
            ct_locality: "Madrid".into(),
            ct_province: "Madrid".into(),
            ct_org: String::new(),
            ct_orgunit: "IT".into(),
            ct_days: "365".into(),
            ct_keybits: "2048".into(),
            ct_hash: "sha256".into(),
        }
    }
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
    folders: Vec<Folder>,
    selected_folder: Option<i64>,
    selected: Option<usize>,
    opened: Option<Value>,
    body_text: Option<String>,
    body_bytes: Option<Vec<u8>>,
    body_note: Option<String>,
    partners: Vec<Value>,

    // Maintenance modules
    view: View,
    certificates: Vec<Value>,
    maint_sel: Option<usize>,
    maint_detail: Option<Value>,
    maint_search: String,
    forms: Forms,
    confirm_delete: Option<Value>,

    // UI
    search: String,
    sort_key: SortKey,
    sort_asc: bool,
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
            folders: Vec::new(),
            selected_folder: None,
            selected: None,
            opened: None,
            body_text: None,
            body_bytes: None,
            body_note: None,
            partners: Vec::new(),
            view: View::Mail,
            certificates: Vec::new(),
            maint_sel: None,
            maint_detail: None,
            maint_search: String::new(),
            forms: Forms::default(),
            confirm_delete: None,
            search: String::new(),
            sort_key: SortKey::Date,
            sort_asc: false,
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

    fn load_certificates(&mut self) {
        let Some(c) = self.client.clone() else { return };
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        self.inflight += 1;
        self.rt.spawn(async move {
            let r = c.certificates.list().await;
            let _ = tx.send(Event::Certificates(r));
            ctx.request_repaint();
        });
    }

    /// Switch the active module, loading its data on demand.
    fn set_view(&mut self, view: View) {
        if self.view == view {
            return;
        }
        self.view = view;
        self.maint_sel = None;
        self.maint_detail = None;
        match view {
            View::Stations if self.stations.is_empty() => self.load_stations(),
            View::Partners if self.partners.is_empty() => self.load_partners(),
            View::Certificates => self.load_certificates(),
            _ => {}
        }
    }

    fn refresh_view(&mut self) {
        match self.view {
            View::Mail => self.refresh_messages(),
            View::Stations => self.load_stations(),
            View::Partners => self.load_partners(),
            View::Certificates => self.load_certificates(),
        }
    }

    fn begin_edit(&mut self, kind: NewKind, item: &Value) {
        self.forms = Forms::default();
        self.forms.open = Some(kind);
        self.forms.edit_id = item.get("id").cloned();
        match kind {
            NewKind::Station => {
                self.forms.st_name = sfield(item, &["name", "nombre"]);
                self.forms.st_as2 = sfield(item, &["as2_id", "as2id"]);
                self.forms.st_email = sfield(item, &["email"]);
            }
            NewKind::Partner => {
                self.forms.pt_name = sfield(item, &["name", "nombre"]);
                self.forms.pt_as2 = sfield(item, &["as2_id", "as2id"]);
                self.forms.pt_email = sfield(item, &["email"]);
                self.forms.pt_url = sfield(item, &["url"]);
            }
            NewKind::Certificate => {}
        }
    }

    fn do_delete(&mut self, id: Value) {
        let Some(c) = self.client.clone() else { return };
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        let view = self.view;
        self.inflight += 1;
        self.rt.spawn(async move {
            let r = match view {
                View::Partners => c.partners.delete(id).await,
                _ => c.stations.delete(id).await,
            };
            let _ = tx.send(Event::Created {
                label: "Deleted".into(),
                res: r,
            });
            ctx.request_repaint();
        });
    }

    fn submit_create(&mut self) {
        let Some(kind) = self.forms.open else { return };
        let Some(c) = self.client.clone() else { return };
        let editing = self.forms.edit_id.clone();
        let (mut body, label): (Value, &'static str) = match kind {
            NewKind::Station => (
                json!({
                    "name": self.forms.st_name.trim(),
                    "as2_id": self.forms.st_as2.trim(),
                    "email": self.forms.st_email.trim(),
                }),
                if editing.is_some() {
                    "Station updated"
                } else {
                    "Station created"
                },
            ),
            NewKind::Partner => {
                let station_id = self
                    .stations
                    .get(self.forms.pt_station)
                    .and_then(|s| s.get("id"))
                    .cloned()
                    .unwrap_or(Value::Null);
                (
                    json!({
                        "station": station_id,
                        "name": self.forms.pt_name.trim(),
                        "as2_id": self.forms.pt_as2.trim(),
                        "email": self.forms.pt_email.trim(),
                        "url": self.forms.pt_url.trim(),
                    }),
                    if editing.is_some() {
                        "Partner updated"
                    } else {
                        "Partner created"
                    },
                )
            }
            NewKind::Certificate => {
                let station_id = self
                    .stations
                    .get(self.forms.ct_station)
                    .and_then(|s| s.get("id"))
                    .cloned()
                    .unwrap_or(Value::Null);
                (
                    json!({
                        "self_signed": true,
                        "station": station_id,
                        "commonName": self.forms.ct_cn.trim(),
                        "email": self.forms.ct_email.trim(),
                        "countryName": self.forms.ct_country.trim(),
                        "localityName": self.forms.ct_locality.trim(),
                        "provinceName": self.forms.ct_province.trim(),
                        "organization": self.forms.ct_org.trim(),
                        "organizationUnitName": self.forms.ct_orgunit.trim(),
                        "dias": self.forms.ct_days.trim().parse::<i64>().unwrap_or(365),
                        "keybits": self.forms.ct_keybits.trim().parse::<i64>().unwrap_or(2048),
                        "hash": self.forms.ct_hash.trim(),
                    }),
                    "Certificate created",
                )
            }
        };
        // In edit mode, add the id and route to the update endpoint.
        if let Some(id) = editing.clone() {
            if let Some(obj) = body.as_object_mut() {
                obj.insert("id".into(), id);
            }
        }
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        self.forms.busy = true;
        self.inflight += 1;
        self.rt.spawn(async move {
            let r = match (kind, editing.is_some()) {
                (NewKind::Station, false) => c.stations.create(body).await,
                (NewKind::Station, true) => c.stations.update(body).await,
                (NewKind::Partner, false) => c.partners.create(body).await,
                (NewKind::Partner, true) => c.partners.update(body).await,
                (NewKind::Certificate, _) => c.certificates.create(body).await,
            };
            let _ = tx.send(Event::Created {
                label: label.into(),
                res: r,
            });
            ctx.request_repaint();
        });
    }

    fn refresh_messages(&mut self) {
        let Some(c) = self.client.clone() else { return };
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        let mut body = json!({ "limit": 500 });
        if let Some(sid) = self.selected_station {
            body["station"] = json!(sid);
        }
        if let Some(fid) = self.selected_folder {
            body["folder"] = json!(fid);
        }
        self.inflight += 1;
        self.status = "Loading messages…".into();
        self.rt.spawn(async move {
            let r = c.messages.list(body).await;
            let _ = tx.send(Event::Messages(r));
            ctx.request_repaint();
        });
    }

    /// Load the folder tree for the selected station (or the whole site).
    fn load_folders(&mut self) {
        let Some(c) = self.client.clone() else { return };
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        let mut body = json!({});
        if let Some(sid) = self.selected_station {
            body["station"] = json!(sid);
        }
        self.rt.spawn(async move {
            let r = c.messages.folders(body).await;
            let _ = tx.send(Event::Folders(r));
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

    /// Save the current payload, letting the user pick the destination.
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
        if let Some(dest) = rfd::FileDialog::new()
            .set_title("Save payload")
            .set_directory(downloads_dir())
            .set_file_name(format!("{safe}.edi"))
            .add_filter("EDI / EDIFACT", &["edi", "txt", "dat", "xml"])
            .add_filter("All files", &["*"])
            .save_file()
        {
            match std::fs::write(&dest, &bytes) {
                Ok(_) => self.status = format!("Saved → {}", dest.display()),
                Err(e) => self.error = Some(format!("Save failed: {e}")),
            }
        }
    }

    fn logout(&mut self) {
        self.client = None;
        self.screen = Screen::Login;
        self.messages.clear();
        self.stations.clear();
        self.partners.clear();
        self.folders.clear();
        self.selected_folder = None;
        self.selected = None;
        self.opened = None;
        self.body_text = None;
        self.body_bytes = None;
        self.status.clear();
    }

    /// Fallback: rebuild the folder list from the loaded messages when the
    /// `/messages/folders` endpoint is unavailable (older nodes).
    fn derive_folders(&mut self) {
        let mut folders: Vec<Folder> = Vec::new();
        for m in &self.messages {
            let Some(id) = m.get("folder_id").and_then(|v| v.as_i64()) else {
                continue;
            };
            if let Some(f) = folders.iter_mut().find(|f| f.id == id) {
                f.count += 1;
            } else {
                let name = sfield(m, &["folder_name"]);
                folders.push(Folder {
                    id,
                    name: if name.is_empty() {
                        format!("Folder {id}")
                    } else {
                        name
                    },
                    parent: None,
                    count: 1,
                });
            }
        }
        folders.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        self.set_folders(folders);
    }

    fn set_folders(&mut self, folders: Vec<Folder>) {
        if let Some(sel) = self.selected_folder {
            if !folders.iter().any(|f| f.id == sel) {
                self.selected_folder = None;
            }
        }
        self.folders = folders;
    }

    /// Depth-first display order of the folder tree: (folder index, depth).
    fn folder_order(&self) -> Vec<(usize, u8)> {
        let mut out = Vec::new();
        self.push_children(None, 0, &mut out);
        // Any orphans (parent not in the set) are shown at the root.
        for (i, f) in self.folders.iter().enumerate() {
            if !out.iter().any(|(j, _)| *j == i)
                && f.parent
                    .map(|p| !self.folders.iter().any(|g| g.id == p))
                    .unwrap_or(false)
            {
                out.push((i, 0));
            }
        }
        out
    }

    fn push_children(&self, parent: Option<i64>, depth: u8, out: &mut Vec<(usize, u8)>) {
        for (i, f) in self.folders.iter().enumerate() {
            if f.parent == parent {
                out.push((i, depth));
                self.push_children(Some(f.id), depth + 1, out);
            }
        }
    }

    /// Indices of the messages to show, filtered by folder + search and sorted.
    fn visible_indices(&self) -> Vec<usize> {
        let needle = self.search.to_lowercase();
        let mut idx: Vec<usize> = self
            .messages
            .iter()
            .enumerate()
            .filter(|(_, m)| {
                if needle.is_empty() {
                    return true;
                }
                sfield(m, &["subject", "asunto"])
                    .to_lowercase()
                    .contains(&needle)
                    || sfield(m, &["partner_name", "socio_nombre"])
                        .to_lowercase()
                        .contains(&needle)
            })
            .map(|(i, _)| i)
            .collect();

        let key = self.sort_key;
        idx.sort_by(|&a, &b| {
            let ma = &self.messages[a];
            let mb = &self.messages[b];
            let ord = match key {
                SortKey::Subject => sfield(ma, &["subject", "asunto"])
                    .to_lowercase()
                    .cmp(&sfield(mb, &["subject", "asunto"]).to_lowercase()),
                SortKey::Partner => sfield(ma, &["partner_name", "socio_nombre"])
                    .to_lowercase()
                    .cmp(&sfield(mb, &["partner_name", "socio_nombre"]).to_lowercase()),
                SortKey::Date => {
                    sfield(ma, &["date", "fecha"]).cmp(&sfield(mb, &["date", "fecha"]))
                }
                SortKey::Mdn => sfield(ma, &["mdn"]).cmp(&sfield(mb, &["mdn"])),
            };
            if self.sort_asc {
                ord
            } else {
                ord.reverse()
            }
        });
        idx
    }

    fn set_sort(&mut self, key: SortKey) {
        if self.sort_key == key {
            self.sort_asc = !self.sort_asc;
        } else {
            self.sort_key = key;
            self.sort_asc = !matches!(key, SortKey::Date);
        }
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
                                self.load_folders();
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
                Event::Folders(r) => match r {
                    Ok(v) if !v.is_empty() => {
                        let folders = v.iter().map(folder_from).collect();
                        self.set_folders(folders);
                    }
                    // No endpoint (older node) or no folders → fall back to
                    // whatever the loaded messages reveal.
                    _ => self.derive_folders(),
                },
                Event::Messages(r) => {
                    self.inflight = self.inflight.saturating_sub(1);
                    match r {
                        Ok(v) => {
                            self.status = format!("{} messages", v.len());
                            self.messages = v;
                            if self.folders.is_empty() {
                                self.derive_folders();
                            }
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
                Event::Certificates(r) => {
                    self.inflight = self.inflight.saturating_sub(1);
                    match r {
                        Ok(v) => {
                            self.status = format!("{} certificates", v.len());
                            self.certificates = v;
                        }
                        Err(e) => self.error = Some(e.to_string()),
                    }
                }
                Event::MaintDetail(r) => {
                    self.inflight = self.inflight.saturating_sub(1);
                    match r {
                        Ok(v) => self.maint_detail = Some(v),
                        Err(e) => self.error = Some(e.to_string()),
                    }
                }
                Event::Created { label, res } => {
                    self.inflight = self.inflight.saturating_sub(1);
                    self.forms.busy = false;
                    match res {
                        Ok(_) => {
                            self.forms = Forms::default();
                            self.maint_sel = None;
                            self.maint_detail = None;
                            self.status = format!("{label} ✓");
                            self.refresh_view();
                        }
                        Err(e) => self.error = Some(format!("{label} failed: {e}")),
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
                    icons::show(ui, Icon::Logo, 32.0);
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
        self.ui_nav(ctx);
        match self.view {
            View::Mail => self.ui_mail(ctx),
            _ => self.ui_maint(ctx),
        }
    }

    fn ui_mail(&mut self, ctx: &egui::Context) {
        self.ui_toolbar(ctx);
        self.ui_statusbar(ctx);
        self.ui_folders(ctx);
        if self.selected.is_some() {
            self.ui_detail(ctx);
        }
        self.ui_grid(ctx);
        if self.compose_open {
            self.ui_compose(ctx);
        }
    }

    /// Leftmost icon rail to switch modules.
    fn ui_nav(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("nav")
            .exact_width(72.0)
            .resizable(false)
            .frame(
                egui::Frame::default()
                    .fill(Color32::from_rgb(0x1E, 0x2A, 0x38))
                    .inner_margin(egui::Margin::symmetric(6.0, 10.0)),
            )
            .show(ctx, |ui| {
                let mut go: Option<View> = None;
                for (view, icon, label) in [
                    (View::Mail, Icon::Message, "Mail"),
                    (View::Stations, Icon::Station, "Stations"),
                    (View::Partners, Icon::Partner, "Partners"),
                    (View::Certificates, Icon::Certificate, "Certificates"),
                ] {
                    if nav_item(ui, icon, label, self.view == view) {
                        go = Some(view);
                    }
                    ui.add_space(4.0);
                }
                if let Some(v) = go {
                    self.set_view(v);
                }
            });
    }

    fn ui_maint(&mut self, ctx: &egui::Context) {
        self.ui_maint_toolbar(ctx);
        self.ui_statusbar(ctx);
        if self.maint_sel.is_some() {
            self.ui_maint_detail(ctx);
        }
        self.ui_maint_grid(ctx);
        if self.forms.open.is_some() {
            self.ui_create(ctx);
        }
        if self.confirm_delete.is_some() {
            self.ui_confirm_delete(ctx);
        }
    }

    fn ui_confirm_delete(&mut self, ctx: &egui::Context) {
        let mut do_it = false;
        let mut cancel = false;
        egui::Window::new(RichText::new("  Confirm delete").strong())
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    icons::show(ui, Icon::Warning, 20.0);
                    ui.label("This action cannot be undone. Delete the selected item?");
                });
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui
                        .add(
                            egui::Button::image_and_text(
                                icons::image(Icon::Delete, 16.0),
                                RichText::new("Delete").strong(),
                            )
                            .fill(DANGER),
                        )
                        .clicked()
                    {
                        do_it = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            });
        if do_it {
            if let Some(id) = self.confirm_delete.take() {
                self.do_delete(id);
                self.maint_sel = None;
                self.maint_detail = None;
            }
        } else if cancel {
            self.confirm_delete = None;
        }
    }

    fn ui_maint_toolbar(&mut self, ctx: &egui::Context) {
        let (title, new_label, new_kind) = match self.view {
            View::Stations => ("Stations", "New station", NewKind::Station),
            View::Partners => ("Partners", "New partner", NewKind::Partner),
            View::Certificates => ("Certificates", "New certificate", NewKind::Certificate),
            View::Mail => ("Mail", "New", NewKind::Station),
        };
        egui::TopBottomPanel::top("maint_toolbar")
            .frame(
                egui::Frame::default()
                    .fill(CARD)
                    .stroke(Stroke::new(1.0, BORDER))
                    .inner_margin(egui::Margin::symmetric(10.0, 7.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(title).size(16.0).strong().color(TEXT));
                    ui.add_space(8.0);
                    if icons::labeled_button(ui, Icon::Add, 16.0, new_label).clicked() {
                        if self.stations.is_empty() {
                            self.load_stations();
                        }
                        self.forms = Forms::default();
                        self.forms.open = Some(new_kind);
                    }
                    if icons::labeled_button(ui, Icon::Refresh, 16.0, "Refresh").clicked() {
                        self.refresh_view();
                    }
                    ui.separator();
                    icons::show(ui, Icon::Search, 16.0);
                    ui.add(
                        egui::TextEdit::singleline(&mut self.maint_search)
                            .hint_text("Search")
                            .desired_width(220.0),
                    );
                    if !self.maint_search.is_empty() && ui.small_button("✕").clicked() {
                        self.maint_search.clear();
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if self.inflight > 0 {
                            ui.spinner();
                        }
                    });
                });
            });

        if let Some(err) = self.error.clone() {
            egui::TopBottomPanel::top("maint_err")
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

    fn ui_maint_grid(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(egui::Frame::default().fill(CARD).inner_margin(0.0))
            .show(ctx, |ui| {
                let view = self.view;
                let headers = maint_headers(view);
                let needle = self.maint_search.to_lowercase();
                let rows: Vec<(usize, Value, Vec<String>)> = self
                    .current_maint_list()
                    .iter()
                    .enumerate()
                    .map(|(i, v)| {
                        (
                            i,
                            v.get("id").cloned().unwrap_or(Value::Null),
                            maint_cells(view, v),
                        )
                    })
                    .filter(|(_, _, cells)| {
                        needle.is_empty()
                            || cells.iter().any(|c| c.to_lowercase().contains(&needle))
                    })
                    .collect();

                if rows.is_empty() {
                    ui.add_space(48.0);
                    ui.vertical_centered(|ui| {
                        icons::show(ui, Icon::Add, 24.0);
                        ui.label(RichText::new("Nothing here yet").color(MUTED));
                        ui.label(
                            RichText::new("Use the New button to create one.")
                                .small()
                                .color(MUTED),
                        );
                    });
                    return;
                }

                let clicked: Cell<Option<(usize, Value)>> = Cell::new(None);
                let sel = self.maint_sel;
                let mut table = TableBuilder::new(ui)
                    .striped(true)
                    .resizable(true)
                    .sense(egui::Sense::click())
                    .cell_layout(Layout::left_to_right(Align::Center));
                for _ in &headers {
                    table = table.column(Column::remainder().at_least(90.0).clip(true));
                }
                table
                    .header(24.0, |mut header| {
                        for h in &headers {
                            header.col(|ui| {
                                ui.label(RichText::new(*h).strong());
                            });
                        }
                    })
                    .body(|body| {
                        body.rows(28.0, rows.len(), |mut row| {
                            let (idx, id, cells) = &rows[row.index()];
                            row.set_selected(sel == Some(*idx));
                            for cell in cells {
                                row.col(|ui| {
                                    ui.label(cell);
                                });
                            }
                            if row.response().clicked() {
                                clicked.set(Some((*idx, id.clone())));
                            }
                        });
                    });

                if let Some((i, id)) = clicked.take() {
                    self.open_maint_by_id(i, id);
                }
            });
    }

    fn ui_maint_detail(&mut self, ctx: &egui::Context) {
        let (icon, title) = match self.view {
            View::Stations => (Icon::Station, "Station"),
            View::Partners => (Icon::Partner, "Partner"),
            View::Certificates => (Icon::Certificate, "Certificate"),
            View::Mail => (Icon::Message, "Item"),
        };
        egui::SidePanel::right("maint_detail")
            .resizable(true)
            .default_width(420.0)
            .frame(
                egui::Frame::default()
                    .fill(CARD)
                    .stroke(Stroke::new(1.0, BORDER))
                    .inner_margin(14.0),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    icons::show(ui, icon, 20.0);
                    ui.label(RichText::new(title).size(16.0).strong().color(TEXT));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.small_button("Close").clicked() {
                            self.maint_sel = None;
                            self.maint_detail = None;
                        }
                    });
                });
                ui.separator();
                // Edit / Delete are available for stations and partners.
                if matches!(self.view, View::Stations | View::Partners) {
                    if let Some(item) = self.maint_detail.clone() {
                        let kind = if self.view == View::Partners {
                            NewKind::Partner
                        } else {
                            NewKind::Station
                        };
                        ui.horizontal(|ui| {
                            if icons::labeled_button(ui, Icon::Add, 16.0, "Edit").clicked() {
                                if self.stations.is_empty() {
                                    self.load_stations();
                                }
                                self.begin_edit(kind, &item);
                            }
                            if icons::labeled_button(ui, Icon::Delete, 16.0, "Delete").clicked() {
                                self.confirm_delete = item.get("id").cloned();
                            }
                        });
                        ui.separator();
                    }
                }
                match self.maint_detail.clone() {
                    None => {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label("Loading…");
                        });
                    }
                    Some(v) => {
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            egui::Grid::new("maint_detail_grid")
                                .num_columns(2)
                                .spacing([10.0, 6.0])
                                .show(ui, |ui| {
                                    if let Some(obj) = v.as_object() {
                                        for (k, val) in obj {
                                            if k == "raw" || val.is_null() {
                                                continue;
                                            }
                                            ui.label(RichText::new(pretty_key(k)).color(MUTED));
                                            ui.label(value_str(val));
                                            ui.end_row();
                                        }
                                    }
                                });
                        });
                    }
                }
            });
    }

    fn ui_create(&mut self, ctx: &egui::Context) {
        let Some(kind) = self.forms.open else { return };
        let editing = self.forms.edit_id.is_some();
        let title = match (kind, editing) {
            (NewKind::Station, false) => "New station",
            (NewKind::Station, true) => "Edit station",
            (NewKind::Partner, false) => "New partner",
            (NewKind::Partner, true) => "Edit partner",
            (NewKind::Certificate, _) => "New certificate (self-signed)",
        };
        let mut open = true;
        let mut submit = false;
        egui::Window::new(RichText::new(format!("  {title}")).strong())
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(460.0)
            .show(ctx, |ui| {
                egui::Grid::new("create_grid")
                    .num_columns(2)
                    .spacing([10.0, 8.0])
                    .show(ui, |ui| match kind {
                        NewKind::Station => {
                            field(ui, "Name", &mut self.forms.st_name);
                            field(ui, "AS2 ID", &mut self.forms.st_as2);
                            field(ui, "Email", &mut self.forms.st_email);
                        }
                        NewKind::Partner => {
                            if !editing {
                                station_combo(ui, &self.stations, &mut self.forms.pt_station);
                            }
                            field(ui, "Name", &mut self.forms.pt_name);
                            field(ui, "AS2 ID", &mut self.forms.pt_as2);
                            field(ui, "Email", &mut self.forms.pt_email);
                            field(ui, "Endpoint URL", &mut self.forms.pt_url);
                        }
                        NewKind::Certificate => {
                            station_combo(ui, &self.stations, &mut self.forms.ct_station);
                            field(ui, "Common name", &mut self.forms.ct_cn);
                            field(ui, "Email", &mut self.forms.ct_email);
                            field(ui, "Country", &mut self.forms.ct_country);
                            field(ui, "Locality", &mut self.forms.ct_locality);
                            field(ui, "Province", &mut self.forms.ct_province);
                            field(ui, "Organization", &mut self.forms.ct_org);
                            field(ui, "Org. unit", &mut self.forms.ct_orgunit);
                            field(ui, "Days", &mut self.forms.ct_days);
                            field(ui, "Key bits", &mut self.forms.ct_keybits);
                            field(ui, "Hash", &mut self.forms.ct_hash);
                        }
                    });

                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    let busy = self.forms.busy;
                    let create = ui.add_enabled(
                        !busy,
                        egui::Button::image_and_text(
                            icons::image(Icon::Add, 16.0),
                            RichText::new(if editing { "Update" } else { "Create" }).strong(),
                        )
                        .fill(ACCENT),
                    );
                    if create.clicked() {
                        submit = true;
                    }
                    if busy {
                        ui.spinner();
                    }
                });
            });
        if submit {
            self.submit_create();
        }
        if !open && !self.forms.busy {
            self.forms.open = None;
        }
    }

    fn current_maint_list(&self) -> &[Value] {
        match self.view {
            View::Partners => &self.partners,
            View::Certificates => &self.certificates,
            _ => &self.stations,
        }
    }

    fn open_maint_by_id(&mut self, index: usize, id: Value) {
        self.maint_sel = Some(index);
        self.maint_detail = None;
        if id.is_null() {
            return;
        }
        let Some(c) = self.client.clone() else { return };
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        let view = self.view;
        self.inflight += 1;
        self.rt.spawn(async move {
            let r = match view {
                View::Partners => c.partners.get(id).await,
                View::Certificates => c.certificates.get(id).await,
                _ => c.stations.get(id).await,
            };
            let _ = tx.send(Event::MaintDetail(r));
            ctx.request_repaint();
        });
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

                    // Station picker (like the web toolbar combo).
                    icons::show(ui, Icon::Station, 16.0);
                    let current = self
                        .selected_station
                        .and_then(|sid| {
                            self.stations
                                .iter()
                                .find(|s| s.get("id").and_then(|v| v.as_i64()) == Some(sid))
                        })
                        .map(|s| sfield(s, &["name", "nombre"]))
                        .unwrap_or_else(|| "All stations".into());
                    let mut change: Option<Option<i64>> = None;
                    egui::ComboBox::from_id_salt("station")
                        .selected_text(current)
                        .width(240.0)
                        .show_ui(ui, |ui| {
                            if ui
                                .selectable_label(self.selected_station.is_none(), "All stations")
                                .clicked()
                            {
                                change = Some(None);
                            }
                            for s in &self.stations {
                                let id = s.get("id").and_then(|v| v.as_i64());
                                let name = sfield(s, &["name", "nombre"]);
                                if ui
                                    .selectable_label(self.selected_station == id, name)
                                    .clicked()
                                {
                                    change = Some(id);
                                }
                            }
                        });
                    if let Some(sel) = change {
                        self.selected_station = sel;
                        self.selected_folder = None;
                        self.load_folders();
                        self.refresh_messages();
                    }

                    ui.separator();
                    icons::show(ui, Icon::Search, 16.0);
                    ui.add(
                        egui::TextEdit::singleline(&mut self.search)
                            .hint_text("Search subject or partner")
                            .desired_width(200.0),
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

    fn ui_folders(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("folders")
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
                    ui.label(RichText::new("Folders").strong().color(TEXT));
                });
                ui.add_space(6.0);

                let all_count: usize = self.folders.iter().map(|f| f.count).sum();
                let mut change: Option<Option<i64>> = None;
                if tree_row(
                    ui,
                    Icon::Inbox,
                    "All messages",
                    Some(all_count),
                    0,
                    self.selected_folder.is_none(),
                )
                .clicked()
                {
                    change = Some(None);
                }
                let order = self.folder_order();
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for (i, depth) in order {
                        let f = &self.folders[i];
                        let sel = self.selected_folder == Some(f.id);
                        if tree_row(ui, Icon::Folder, &f.name, Some(f.count), depth, sel).clicked()
                        {
                            change = Some(Some(f.id));
                        }
                    }
                    if self.folders.is_empty() {
                        ui.add_space(10.0);
                        ui.label(RichText::new("No folders yet").small().color(MUTED));
                    }
                });
                if let Some(sel) = change {
                    self.selected_folder = sel;
                    self.selected = None;
                    self.opened = None;
                    self.refresh_messages();
                }
            });
    }

    fn ui_grid(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(egui::Frame::default().fill(CARD).inner_margin(0.0))
            .show(ctx, |ui| {
                let visible = self.visible_indices();
                if visible.is_empty() {
                    ui.add_space(48.0);
                    ui.vertical_centered(|ui| {
                        icons::show(ui, Icon::Inbox, 24.0);
                        ui.label(RichText::new("No messages").color(MUTED));
                        ui.label(
                            RichText::new("Pick a station, a folder, or press Receive.")
                                .small()
                                .color(MUTED),
                        );
                    });
                    return;
                }

                let clicked: Cell<Option<usize>> = Cell::new(None);
                let sort_click: Cell<Option<SortKey>> = Cell::new(None);
                let arrow = |k: SortKey| -> &'static str {
                    if self.sort_key == k {
                        if self.sort_asc {
                            " ▲"
                        } else {
                            " ▼"
                        }
                    } else {
                        ""
                    }
                };

                TableBuilder::new(ui)
                    .striped(true)
                    .resizable(true)
                    .sense(egui::Sense::click())
                    .cell_layout(Layout::left_to_right(Align::Center))
                    .column(Column::exact(34.0))
                    .column(Column::remainder().at_least(180.0).clip(true))
                    .column(Column::initial(180.0).at_least(90.0).clip(true))
                    .column(Column::initial(126.0).at_least(90.0))
                    .column(Column::initial(70.0).at_least(50.0))
                    .header(24.0, |mut header| {
                        header.col(|_ui| {});
                        header.col(|ui| {
                            if ui
                                .button(
                                    RichText::new(format!("Subject{}", arrow(SortKey::Subject)))
                                        .strong(),
                                )
                                .clicked()
                            {
                                sort_click.set(Some(SortKey::Subject));
                            }
                        });
                        header.col(|ui| {
                            if ui
                                .button(
                                    RichText::new(format!("Partner{}", arrow(SortKey::Partner)))
                                        .strong(),
                                )
                                .clicked()
                            {
                                sort_click.set(Some(SortKey::Partner));
                            }
                        });
                        header.col(|ui| {
                            if ui
                                .button(
                                    RichText::new(format!("Date{}", arrow(SortKey::Date))).strong(),
                                )
                                .clicked()
                            {
                                sort_click.set(Some(SortKey::Date));
                            }
                        });
                        header.col(|ui| {
                            if ui
                                .button(
                                    RichText::new(format!("MDN{}", arrow(SortKey::Mdn))).strong(),
                                )
                                .clicked()
                            {
                                sort_click.set(Some(SortKey::Mdn));
                            }
                        });
                    })
                    .body(|body| {
                        body.rows(28.0, visible.len(), |mut row| {
                            let i = visible[row.index()];
                            let m = &self.messages[i];
                            row.set_selected(self.selected == Some(i));
                            let incoming = bool_dir(m);
                            row.col(|ui| {
                                icons::show(ui, if incoming { Icon::In } else { Icon::Out }, 16.0);
                            });
                            row.col(|ui| {
                                ui.label(sfield(m, &["subject", "asunto"]));
                            });
                            row.col(|ui| {
                                ui.label(sfield(m, &["partner_name", "socio_nombre"]));
                            });
                            row.col(|ui| {
                                ui.label(
                                    RichText::new(short_date(&sfield(m, &["date", "fecha"])))
                                        .color(MUTED),
                                );
                            });
                            row.col(|ui| {
                                let mdn = sfield(m, &["mdn"]);
                                if !mdn.is_empty() {
                                    let ok = mdn.eq_ignore_ascii_case("ok");
                                    ui.colored_label(if ok { OK_GREEN } else { DANGER }, mdn);
                                }
                            });
                            if row.response().clicked() {
                                clicked.set(Some(i));
                            }
                        });
                    });

                if let Some(k) = sort_click.get() {
                    self.set_sort(k);
                }
                if let Some(i) = clicked.get() {
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

                let incoming = bool_dir(&m);
                ui.horizontal(|ui| {
                    icons::show(ui, if incoming { Icon::In } else { Icon::Out }, 18.0);
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
                    if icons::labeled_button(ui, Icon::Save, 16.0, "Download…").clicked() {
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
            .default_width(500.0)
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
                            .width(340.0)
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
                                .desired_width(340.0),
                        );
                        ui.end_row();

                        label_with_icon(ui, Icon::Attach, "File");
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut self.compose_file)
                                    .hint_text("choose or drag a file")
                                    .desired_width(258.0),
                            );
                            if ui.button("Browse…").clicked() {
                                if let Some(p) = rfd::FileDialog::new()
                                    .set_title("Choose a file to send")
                                    .pick_file()
                                {
                                    self.compose_file = p.display().to_string();
                                }
                            }
                        });
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

/// A folder-tree entry: icon, name, and an optional count badge on the right.
fn tree_row(
    ui: &mut egui::Ui,
    icon: Icon,
    name: &str,
    count: Option<usize>,
    depth: u8,
    selected: bool,
) -> egui::Response {
    let height = 30.0;
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
    let indent = 8.0 + f32::from(depth) * 14.0;
    let icon_rect = Rect::from_min_size(
        egui::pos2(rect.left() + indent, rect.center().y - 9.0),
        Vec2::splat(18.0),
    );
    icons::image(icon, 18.0).paint_at(ui, icon_rect);
    let name_color = if selected { ACCENT } else { TEXT };
    let p = ui.painter();
    p.text(
        egui::pos2(icon_rect.right() + 8.0, rect.center().y),
        Align2::LEFT_CENTER,
        trunc(name, 22),
        FontId::proportional(13.5),
        name_color,
    );
    if let Some(c) = count {
        p.text(
            egui::pos2(rect.right() - 8.0, rect.center().y),
            Align2::RIGHT_CENTER,
            c.to_string(),
            FontId::proportional(11.5),
            MUTED,
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

/// A nav-rail item: stacked icon over label, highlighted when active.
fn nav_item(ui: &mut egui::Ui, icon: Icon, label: &str, active: bool) -> bool {
    let (rect, resp) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 52.0), egui::Sense::click());
    if active {
        ui.painter()
            .rect_filled(rect, 6.0, Color32::from_rgb(0x2C, 0x3E, 0x50));
        ui.painter().rect_filled(
            Rect::from_min_size(rect.left_top(), Vec2::new(3.0, rect.height())),
            0.0,
            Color32::from_rgb(0x4D, 0xA3, 0xF0),
        );
    } else if resp.hovered() {
        ui.painter()
            .rect_filled(rect, 6.0, Color32::from_rgb(0x27, 0x35, 0x45));
    }
    let icon_rect = Rect::from_min_size(
        egui::pos2(rect.center().x - 9.0, rect.top() + 8.0),
        Vec2::splat(18.0),
    );
    icons::image(icon, 18.0).paint_at(ui, icon_rect);
    let color = if active {
        Color32::WHITE
    } else {
        Color32::from_rgb(0xB9, 0xC3, 0xCE)
    };
    ui.painter().text(
        egui::pos2(rect.center().x, rect.bottom() - 8.0),
        Align2::CENTER_BOTTOM,
        label,
        FontId::proportional(10.0),
        color,
    );
    resp.clicked()
}

/// Column headers for a maintenance module.
fn maint_headers(view: View) -> Vec<&'static str> {
    match view {
        View::Partners => vec!["Name", "AS2 ID", "Email", "Station"],
        View::Certificates => vec!["Common name", "Email", "Station", "Valid until"],
        _ => vec!["Name", "AS2 ID"],
    }
}

/// Cell values for one row of a maintenance module.
fn maint_cells(view: View, v: &Value) -> Vec<String> {
    match view {
        View::Partners => vec![
            sfield(v, &["name", "nombre"]),
            sfield(v, &["as2_id", "as2id"]),
            sfield(v, &["email"]),
            sfield(v, &["station_name", "estacion"]),
        ],
        View::Certificates => vec![
            sfield(v, &["commonName", "commonname"]),
            sfield(v, &["email"]),
            sfield(v, &["station_name", "estacion"]),
            short_date(&sfield(v, &["validityEnd", "validityend"])),
        ],
        _ => vec![
            sfield(v, &["name", "nombre"]),
            sfield(v, &["as2_id", "as2id"]),
        ],
    }
}

/// A labelled single-line text field row inside a form Grid.
fn field(ui: &mut egui::Ui, label: &str, value: &mut String) {
    ui.label(RichText::new(label).color(MUTED));
    ui.add(egui::TextEdit::singleline(value).desired_width(300.0));
    ui.end_row();
}

/// A station picker row inside a form Grid, storing the selected index.
fn station_combo(ui: &mut egui::Ui, stations: &[Value], selected: &mut usize) {
    ui.label(RichText::new("Station").color(MUTED));
    let current = stations
        .get(*selected)
        .map(|s| sfield(s, &["name", "nombre"]))
        .unwrap_or_else(|| "— pick a station —".into());
    egui::ComboBox::from_id_salt("form_station")
        .selected_text(current)
        .width(300.0)
        .show_ui(ui, |ui| {
            for (i, s) in stations.iter().enumerate() {
                ui.selectable_value(selected, i, sfield(s, &["name", "nombre"]));
            }
        });
    ui.end_row();
}

/// Humanize a JSON key for the detail view (snake/alias → Title Case).
fn pretty_key(k: &str) -> String {
    let mut out = String::new();
    for (i, part) in k.split('_').enumerate() {
        if i > 0 {
            out.push(' ');
        }
        let mut chars = part.chars();
        if let Some(f) = chars.next() {
            out.extend(f.to_uppercase());
            out.push_str(chars.as_str());
        }
    }
    out
}

/// Render a JSON scalar for the detail view.
fn value_str(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Bool(b) => (if *b { "yes" } else { "no" }).to_string(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

// --- helpers -----------------------------------------------------------------

/// Build a [`Folder`] from a `/messages/folders` response item.
fn folder_from(v: &Value) -> Folder {
    Folder {
        id: v.get("id").and_then(|x| x.as_i64()).unwrap_or(0),
        name: sfield(v, &["name", "nombre"]),
        parent: v.get("parent_id").and_then(|x| x.as_i64()),
        count: v.get("count").and_then(|x| x.as_u64()).unwrap_or(0) as usize,
    }
}

fn bool_dir(m: &Value) -> bool {
    m.get("incoming")
        .or_else(|| m.get("entrante"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

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
