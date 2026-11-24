//! Application state and top-level wiring.
use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::egui;

use crate::downloads::Downloads;
use crate::images::{ImageCache, ImgSource};
use crate::jobs::Job;
use crate::library::{self, CoverSource, Library, MangaEntry};
use crate::local;
use crate::mangadex::{self, MangaSummary};
use crate::reader::{self, ReaderSession};

#[derive(Clone, PartialEq)]
pub enum Screen {
    Library,
    Search,
    Detail(String),
    History,
    Settings,
}

pub struct Toast {
    pub msg: String,
    pub error: bool,
    pub at: Instant,
}

impl Toast {
    pub fn new(msg: String, error: bool) -> Self {
        Toast { msg, error, at: Instant::now() }
    }
}

pub struct SearchState {
    pub query: String,
    pub results: Vec<MangaSummary>,
    pub total: u32,
    pub job: Option<Job<Result<(Vec<MangaSummary>, u32), String>>>,
    pub loading: bool,
    pub append: bool,
    pub searched_once: bool,
}

impl Default for SearchState {
    fn default() -> Self {
        SearchState {
            query: String::new(),
            results: Vec::new(),
            total: 0,
            job: None,
            loading: false,
            append: false,
            searched_once: false,
        }
    }
}

pub struct DetailState {
    pub manga_id: String,
    pub summary: Option<MangaSummary>,
    pub sum_job: Option<Job<Result<MangaSummary, String>>>,
    pub chapters: Vec<reader::ChapterRef>,
    pub ch_job: Option<Job<Result<Vec<reader::ChapterRef>, String>>>,
}

pub struct App {
    pub lib: Library,
    pub data_dir: PathBuf,
    pub images: ImageCache,
    pub screen: Screen,
    pub reader: Option<ReaderSession>,
    pub search: SearchState,
    pub detail: Option<DetailState>,
    pub downloads: Downloads,
    pub toasts: Vec<Toast>,
    pub save_needed: bool,
    pub local_path_input: String,
    pub dialog_job: Option<Job<Result<String, String>>>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext) -> Self {
        let data_dir = library::data_dir();
        std::fs::create_dir_all(&data_dir).ok();
        let lib = library::load(&data_dir);
        let mut app = App {
            lib,
            data_dir,
            images: ImageCache::new(),
            screen: Screen::Library,
            reader: None,
            search: SearchState::default(),
            detail: None,
            downloads: Downloads::default(),
            toasts: Vec::new(),
            save_needed: false,
            local_path_input: String::new(),
            dialog_job: None,
        };
        app.setup_visuals(&cc.egui_ctx);
        app
    }

    fn setup_visuals(&mut self, ctx: &egui::Context) {
        let mut style = egui::Style::default();
        style.visuals = egui::Visuals::dark();
        style.visuals.widgets.noninteractive.corner_radius = egui::Rounding::same(4);
        style.visuals.widgets.inactive.corner_radius = egui::Rounding::same(6);
        style.visuals.widgets.hovered.corner_radius = egui::Rounding::same(6);
        style.visuals.panel_fill = egui::Color32::from_rgb(18, 18, 24);
        style.visuals.extreme_bg_color = egui::Color32::from_rgb(10, 10, 14);
        ctx.set_style(style);
        ctx.set_theme(egui::Theme::Dark);
    }

    pub fn toast(&mut self, msg: String, error: bool) {
        self.toasts.push(Toast::new(msg, error));
        if self.toasts.len() > 6 {
            self.toasts.remove(0);
        }
    }

    // ---------- navigation ----------

    pub fn open_detail(&mut self, manga_id: String, known: Option<MangaSummary>) {
        let entry = self.lib.entry(&manga_id).cloned();
        let mut ds = DetailState {
            manga_id: manga_id.clone(),
            summary: known,
            sum_job: None,
            chapters: Vec::new(),
            ch_job: None,
        };
        match entry.as_ref().map(|e| &e.source) {
            Some(library::MangaSource::Mangadex { id }) => {
                if ds.summary.is_none() {
                    let id2 = id.clone();
                    ds.sum_job = Some(Job::spawn(move || mangadex::details(&id2)));
                }
                let id2 = id.clone();
                let langs = self.lib.settings.languages.clone();
                let ratings = self.lib.settings.content_ratings.clone();
                ds.ch_job = Some(Job::spawn(move || reader::load_md_chapters(&id2, &langs, &ratings)));
            }
            Some(library::MangaSource::Local { path }) => {
                let p = PathBuf::from(path);
                ds.ch_job = Some(Job::spawn(move || reader::load_local_chapters(&p)));
                if ds.summary.is_none() {
                    ds.summary = Some(MangaSummary {
                        id: manga_id.clone(),
                        title: entry.as_ref().map(|e| e.title.clone()).unwrap_or_default(),
                        cover_url: None,
                        status: String::new(),
                        description: entry.as_ref().map(|e| e.description.clone()).unwrap_or_default(),
                    });
                }
            }
            None => {}
        }
        self.detail = Some(ds);
        self.screen = Screen::Detail(manga_id);
    }

    pub fn open_reader(&mut self, manga_id: String, hint: Option<String>) {
        let title = self
            .lib
            .entry(&manga_id)
            .map(|e| e.title.clone())
            .or_else(|| {
                self.detail
                    .as_ref()
                    .filter(|d| d.manga_id == manga_id)
                    .and_then(|d| d.summary.as_ref().map(|s| s.title.clone()))
            })
            .unwrap_or_else(|| "Манга".into());
        let session = ReaderSession::open(manga_id, title, hint, &self.lib);
        self.reader = Some(session);
    }

    // ---------- local manga import ----------

    pub fn add_local(&mut self, raw_path: &str) {
        let raw = raw_path.trim();
        if raw.is_empty() {
            return;
        }
        let p = match std::fs::canonicalize(raw) {
            Ok(p) => p,
            Err(e) => {
                self.toast(format!("Путь недоступен: {e}"), true);
                return;
            }
        };
        let id = format!("local:{}", p.display());
        if self.lib.contains(&id) {
            self.toast("Эта манга уже в библиотеке".into(), false);
            self.open_detail(id, None);
            return;
        }
        match local::scan(&p) {
            Ok(m) => {
                let cover = m
                    .chapters
                    .first()
                    .and_then(|ch| local::chapter_pages(&ch.path).ok())
                    .and_then(|pages| pages.first().cloned())
                    .map(CoverSource::LocalPage);
                let title = m.title.clone();
                let entry = MangaEntry {
                    id: id.clone(),
                    source: library::MangaSource::Local {
                        path: p.display().to_string(),
                    },
                    title: title.clone(),
                    description: String::new(),
                    status: String::new(),
                    cover,
                    added_at: library::now(),
                    progress: Default::default(),
                    totals: Default::default(),
                    last_chapter: None,
                    bookmarks: Vec::new(),
                    downloaded: Vec::new(),
                    finished: Vec::new(),
                };
                self.lib.add(entry);
                self.save_needed = true;
                self.toast(format!("Добавлено: {title}"), false);
                self.open_detail(id, None);
            }
            Err(e) => self.toast(e, true),
        }
    }

    pub fn browse_local(&mut self) {
        // try zenity / kdialog to open a native picker
        let script = if command_exists("zenity") {
            Some("zenity --file-selection --title='Выберите CBZ или папку с мангой' || zenity --file-selection --directory --title='Или выберите папку'".to_string())
        } else if command_exists("kdialog") {
            Some("kdialog --getopenfilename . '*.cbz *.zip' || kdialog --getexistingdirectory .".to_string())
        } else {
            None
        };
        match script {
            Some(cmd) => {
                self.dialog_job = Some(Job::spawn(move || {
                    let out = std::process::Command::new("sh")
                        .arg("-c")
                        .arg(cmd)
                        .output()
                        .map_err(|e| format!("Диалог не запустился: {e}"))?;
                    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
                    if s.is_empty() {
                        Err("Выбор отменён".to_string())
                    } else {
                        Ok(s)
                    }
                }));
            }
            None => {
                self.toast("Установите zenity для диалога выбора, или введите путь вручную".into(), true);
            }
        }
    }

    // ---------- md add/remove ----------

    pub fn toggle_library(&mut self, s: &MangaSummary) {
        let id = format!("md:{}", s.id);
        if self.lib.contains(&id) {
            self.lib.remove(&id);
            self.toast("Удалено из библиотеки".into(), false);
        } else {
            self.lib.add(MangaEntry::from_summary(s));
            self.toast("Добавлено в библиотеку".into(), false);
        }
        self.save_needed = true;
    }

    // ---------- downloads ----------

    fn poll_downloads(&mut self) {
        let had_queue = self.downloads.active() + self.downloads.pending() > 1;
        for (chapter_id, manga_id, res) in self.downloads.poll(&self.data_dir) {
            match res {
                Ok(()) => {
                    if let Some(e) = self.lib.entry_mut(&manga_id) {
                        if !e.downloaded.contains(&chapter_id) {
                            e.downloaded.push(chapter_id.clone());
                        }
                    }
                    self.save_needed = true;
                    self.toast("Глава скачана и доступна офлайн".into(), false);
                    // reload if this chapter is currently being read
                    if let Some(session) = self.reader.as_mut() {
                        if session
                            .current_chapter()
                            .map(|c| c.key == format!("md:{chapter_id}"))
                            .unwrap_or(false)
                        {
                            session.switch_chapter(session.current);
                        }
                    }
                }
                Err(e) => self.toast(format!("Ошибка загрузки главы: {e}"), true),
            }
        }
        if self.downloads.active() == 0 && self.downloads.pending() == 0 && had_queue {
            self.toast("Все загрузки завершены".into(), false);
        }
    }

    // ---------- update ----------

    pub fn update_inner(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.images.poll(ctx);

        // dialog job
        if self.dialog_job.is_some() {
            let mut job = self.dialog_job.take().unwrap();
            if let Some(res) = job.poll() {
                match res {
                    Ok(path) => self.add_local(&path),
                    Err(e) => {
                        if e != "Выбор отменён" {
                            self.toast(e, true);
                        }
                    }
                }
            } else {
                self.dialog_job = Some(job);
            }
        }
        self.poll_downloads();

        if self.reader.is_some() {
            let App {
                lib,
                data_dir,
                images,
                reader,
                downloads: dl,
                toasts,
                save_needed,
                ..
            } = self;
            if let Some(session) = reader.as_mut() {
                let mut on_toast = |msg: String, err: bool| {
                    toasts.push(Toast::new(msg, err));
                };
                crate::reader::show(session, ctx, lib, images, data_dir, dl, &mut on_toast);
                if session.dirty {
                    *save_needed = true;
                    session.dirty = false;
                }
                if session.start_hint.as_deref() == Some("__close__") {
                    *reader = None;
                }
            }
        } else {
            self.show_main_ui(ctx);
        }

        self.draw_toasts(ctx);

        if self.save_needed {
            self.save_needed = false;
            library::save(&self.data_dir, &self.lib);
        }
        // keep animating (spinners, downloads)
        ctx.request_repaint_after(Duration::from_millis(100));
    }

    fn draw_toasts(&mut self, ctx: &egui::Context) {
        self.toasts
            .retain(|t| t.at.elapsed() < Duration::from_secs(6));
        if self.toasts.is_empty() {
            return;
        }
        let toasts = std::mem::take(&mut self.toasts);
        egui::Area::new(egui::Id::new("toasts"))
            .anchor(egui::Align2::CENTER_BOTTOM, [0.0, -24.0])
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                for t in &toasts {
                    let bg = if t.error {
                        egui::Color32::from_rgb(70, 25, 25)
                    } else {
                        egui::Color32::from_rgb(30, 45, 30)
                    };
                    egui::Frame::default()
                        .fill(bg)
                        .rounding(8.0)
                        .inner_margin(egui::Margin::symmetric(14, 8))
                        .show(ui, |ui| {
                            ui.label(egui::RichText::new(&t.msg).color(egui::Color32::WHITE));
                        });
                    ui.add_space(4.0);
                }
            });
        self.toasts = toasts;
    }
}

fn command_exists(cmd: &str) -> bool {
    std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {cmd}"))
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let t: String = s.chars().take(n).collect();
        format!("{t}…")
    }
}

/// Build the (key, source) pair for a manga cover.
pub fn cover_task(entry: &MangaEntry, data_dir: &std::path::Path) -> Option<(String, ImgSource)> {
    let safe = entry.id.replace([':', '/'], "_");
    match &entry.cover {
        Some(CoverSource::MdUrl(url)) => Some((
            format!("cover:{safe}"),
            ImgSource::UrlDisk {
                url: url.clone(),
                local: data_dir.join("covers").join(format!("{safe}.jpg")),
            },
        )),
        Some(CoverSource::LocalPage(p)) => Some((
            format!("cover:{safe}"),
            ImgSource::Page(p.clone()),
        )),
        None => None,
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.update_inner(ctx, frame)
    }
}
