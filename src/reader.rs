//! Reader: paged / vertical strip / horizontal strip, RTL, zoom & pan,
//! progress tracking, bookmarks, chapter navigation.
use std::cell::RefCell;
use std::path::{Path, PathBuf};

use eframe::egui::{self, ComboBox, Key, Sense, TextureHandle, TopBottomPanel, Vec2};

use crate::downloads;
use crate::images::{ImageCache, ImgSource};
use crate::jobs::Job;
use crate::library::{Bookmark, HistoryItem, Library};
use crate::local;
use crate::mangadex;

#[derive(Clone, Debug)]
pub enum ChapterSource {
    Mangadex { id: String },
    Local(PathBuf),
}

#[derive(Clone, Debug)]
pub struct ChapterRef {
    pub key: String, // "md:{id}" | "local:{path}"
    pub label: String,
    pub source: ChapterSource,
    pub total_pages: u32,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Mode {
    Paged,
    Strip,
    Horizontal,
}

impl Mode {
    pub fn label(&self) -> &'static str {
        match self {
            Mode::Paged => "Постранично",
            Mode::Strip => "Вертикально",
            Mode::Horizontal => "Горизонтально",
        }
    }
    pub fn all() -> [Mode; 3] {
        [Mode::Paged, Mode::Strip, Mode::Horizontal]
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Fit {
    Best,
    Width,
    Height,
    Original,
}

impl Fit {
    pub fn label(&self) -> &'static str {
        match self {
            Fit::Best => "Вписать",
            Fit::Width => "По ширине",
            Fit::Height => "По высоте",
            Fit::Original => "1:1",
        }
    }
    pub fn all() -> [Fit; 4] {
        [Fit::Best, Fit::Width, Fit::Height, Fit::Original]
    }
}

enum PagesState {
    Idle,
    Loading(Job<Result<Vec<ImgSource>, String>>),
    Ready,
    Failed(String),
}

impl PagesState {
    fn is_idle(&self) -> bool {
        matches!(self, PagesState::Idle)
    }
    fn is_ready(&self) -> bool {
        matches!(self, PagesState::Ready)
    }
}

pub struct ReaderSession {
    pub manga_id: String,
    pub manga_title: String,
    pub chapters: Vec<ChapterRef>,
    pub chapters_job: Option<Job<Result<Vec<ChapterRef>, String>>>,
    pub current: usize,
    pub pages: Vec<ImgSource>,
    pub page: usize,
    pub pages_state: PagesState,
    /// chapter key to open at start, or "__close__" to exit
    pub start_hint: Option<String>,
    pub mode: Mode,
    pub fit: Fit,
    pub rtl: bool,
    pub zoom: f32,
    pub pan: Vec2,
    pub wheel_acc: f32,
    pub jump_to: Option<usize>,
    pub pending_save: bool,
    /// library changed and must be saved
    pub dirty: bool,
}

impl ReaderSession {
    pub fn open(
        manga_id: String,
        manga_title: String,
        start_hint: Option<String>,
        lib: &Library,
    ) -> Self {
        let job = match lib.entry(&manga_id).map(|e| &e.source) {
            Some(crate::library::MangaSource::Mangadex { id }) => {
                let id = id.clone();
                let langs = lib.settings.languages.clone();
                let ratings = lib.settings.content_ratings.clone();
                Some(Job::spawn(move || load_md_chapters(&id, &langs, &ratings)))
            }
            Some(crate::library::MangaSource::Local { path }) => {
                let path = PathBuf::from(path);
                Some(Job::spawn(move || load_local_chapters(&path)))
            }
            None => None,
        };
        let s = &lib.settings;
        ReaderSession {
            manga_id,
            manga_title,
            chapters: Vec::new(),
            chapters_job: job,
            current: 0,
            pages: Vec::new(),
            page: 0,
            pages_state: PagesState::Idle,
            start_hint,
            mode: match s.default_mode {
                1 => Mode::Strip,
                2 => Mode::Horizontal,
                _ => Mode::Paged,
            },
            fit: Fit::Best,
            rtl: s.rtl,
            zoom: 1.0,
            pan: Vec2::ZERO,
            wheel_acc: 0.0,
            jump_to: None,
            pending_save: false,
            dirty: false,
        }
    }

    pub fn switch_chapter(&mut self, idx: usize) {
        if idx >= self.chapters.len() {
            return;
        }
        self.current = idx;
        self.pages_state = PagesState::Idle;
        self.pages = Vec::new();
        self.zoom = 1.0;
        self.pan = Vec2::ZERO;
        self.wheel_acc = 0.0;
        self.page = 0;
        self.jump_to = None;
    }

    pub fn current_chapter(&self) -> Option<&ChapterRef> {
        self.chapters.get(self.current)
    }

    pub fn total_pages(&self) -> usize {
        self.pages.len()
    }

    fn turn_page(&mut self, delta: isize) {
        if self.pages.is_empty() {
            return;
        }
        let next = self.page as isize + delta;
        if next < 0 {
            if self.current > 0 {
                let target = self.current - 1;
                self.switch_chapter(target);
                self.page = usize::MAX; // jump to last page once loaded
            }
        } else if next as usize >= self.pages.len() {
            if self.current + 1 < self.chapters.len() {
                self.switch_chapter(self.current + 1);
            } else {
                self.page = self.pages.len() - 1;
                self.pending_save = true;
            }
        } else {
            self.page = next as usize;
            self.pending_save = true;
        }
    }
}

// ---------- background loading ----------

pub fn load_md_chapters(
    id: &str,
    langs: &[String],
    ratings: &[String],
) -> Result<Vec<ChapterRef>, String> {
    let chapters = mangadex::fetch_chapters(id, langs, ratings)?;
    Ok(chapters
        .into_iter()
        .map(|c| ChapterRef {
            key: format!("md:{}", c.id),
            label: c.label,
            source: ChapterSource::Mangadex { id: c.id },
            total_pages: c.pages,
        })
        .collect())
}

pub fn load_local_chapters(path: &Path) -> Result<Vec<ChapterRef>, String> {
    let m = local::scan(path)?;
    Ok(m.chapters
        .iter()
        .map(|ch| {
            let p = &ch.path;
            ChapterRef {
                key: format!("local:{}", p.display()),
                label: ch.title.clone(),
                source: ChapterSource::Local(p.clone()),
                total_pages: 0,
            }
        })
        .collect())
}

fn load_pages(ch: &ChapterRef, data_dir: &Path) -> Result<Vec<ImgSource>, String> {
    match &ch.source {
        ChapterSource::Mangadex { id } => {
            let dir = downloads::chapter_dir(data_dir, id);
            // fully downloaded (offline) -> use disk without touching the network
            if ch.total_pages > 0 && downloads::is_complete(data_dir, id, ch.total_pages) {
                let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
                    .map_err(|e| format!("Папка загрузки: {e}"))?
                    .filter_map(|e| e.ok().map(|e| e.path()))
                    .filter(|p| local::is_image(p))
                    .collect();
                files.sort_by(|a, b| {
                    local::natural_cmp(local::file_name_of(a), local::file_name_of(b))
                });
                return Ok(files
                    .into_iter()
                    .map(|p| ImgSource::Page(local::PageSource::File(p)))
                    .collect());
            }
            // otherwise: per-page hybrid — local file when present, network otherwise
            let (base, hash, pages) = mangadex::at_home(id)?;
            let mut out = Vec::with_capacity(pages.len());
            for (i, p) in pages.iter().enumerate() {
                let ext = p.rsplit('.').next().unwrap_or("jpg").to_ascii_lowercase();
                let local = dir.join(format!("{:04}.{}", i + 1, ext));
                if local.is_file() {
                    out.push(ImgSource::Page(local::PageSource::File(local)));
                } else {
                    out.push(ImgSource::Url(mangadex::page_url(&base, &hash, p)));
                }
            }
            Ok(out)
        }
        ChapterSource::Local(path) => Ok(local::chapter_pages(path)?
            .into_iter()
            .map(ImgSource::Page)
            .collect()),
    }
}

// ---------- main entry ----------

pub fn show(
    session: &mut ReaderSession,
    ctx: &egui::Context,
    lib: &mut Library,
    images: &mut ImageCache,
    data_dir: &Path,
    dl: &mut downloads::Downloads,
    toast: &mut dyn FnMut(String, bool),
) {
    // chapter list job
    if session.chapters_job.is_some() {
        let mut job = session.chapters_job.take().unwrap();
        if let Some(res) = job.poll() {
            match res {
                Ok(chs) => {
                    session.chapters = chs;
                    let start = session
                        .start_hint
                        .as_ref()
                        .and_then(|k| session.chapters.iter().position(|c| &c.key == k))
                        .unwrap_or_else(|| {
                            // default: first not-finished chapter
                            let finished: Vec<String> = lib
                                .entry(&session.manga_id)
                                .map(|e| e.finished.clone())
                                .unwrap_or_default();
                            session
                                .chapters
                                .iter()
                                .position(|c| !finished.contains(&c.key))
                                .unwrap_or(0)
                        });
                    session.current = start;
                    session.pages_state = PagesState::Idle;
                    if session.chapters.is_empty() {
                        let langs = lib.settings.languages.join(", ");
                        (toast)(
                            format!("Нет глав на языках: {langs}. Проверьте настройки."),
                            false,
                        );
                    }
                }
                Err(e) => (toast)(e, true),
            }
        } else {
            session.chapters_job = Some(job);
        }
    }

    // start loading pages
    if session.pages_state.is_idle() {
        if let Some(ch) = session.current_chapter() {
            let ch = ch.clone();
            let dir = data_dir.to_path_buf();
            session.pages_state = PagesState::Loading(Job::spawn(move || load_pages(&ch, &dir)));
        }
    }
    if matches!(session.pages_state, PagesState::Loading(_)) {
        let mut job = match std::mem::replace(&mut session.pages_state, PagesState::Idle) {
            PagesState::Loading(j) => j,
            _ => unreachable!(),
        };
        if let Some(res) = job.poll() {
            match res {
                Ok(pages) => {
                    session.pages = pages;
                    if let Some(ch) = session.current_chapter() {
                        let saved = lib
                            .entry(&session.manga_id)
                            .and_then(|e| e.progress.get(&ch.key).copied())
                            .unwrap_or(0) as usize;
                        let last = session.pages.len().saturating_sub(1);
                        if session.page == usize::MAX {
                            session.page = last;
                        } else if session.page == 0 {
                            session.page = saved.min(last);
                        } else {
                            session.page = session.page.min(last);
                        }
                    }
                    session.pages_state = PagesState::Ready;
                    session.jump_to = Some(session.page);
                }
                Err(e) => session.pages_state = PagesState::Failed(e),
            }
        } else {
            session.pages_state = PagesState::Loading(job);
        }
    }

    handle_input(session, ctx);

    // ---- top bar ----
    TopBottomPanel::top("reader_top")
        .frame(egui::Frame::default().fill(egui::Color32::from_rgb(12, 14, 12)))
        .show(ctx, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui.button("<- Библиотека").clicked() {
                    session.start_hint = Some("__close__".into());
                }
                ui.label(egui::RichText::new(crate::app::truncate(
                    &session.manga_title,
                    40,
                )).strong());
                ui.separator();

                let cur_label = session
                    .current_chapter()
                    .map(|c| crate::app::truncate(&c.label, 46))
                    .unwrap_or_else(|| "Глава…".into());
                let chapter_labels: Vec<String> = session
                    .chapters
                    .iter()
                    .map(|c| crate::app::truncate(&c.label, 56))
                    .collect();
                ComboBox::from_id_salt("reader_chapter")
                    .selected_text(cur_label)
                    .width(220.0)
                    .show_ui(ui, |ui| {
                        for (i, l) in chapter_labels.iter().enumerate() {
                            if ui.selectable_label(i == session.current, l).clicked()
                                && i != session.current
                            {
                                session.switch_chapter(i);
                            }
                        }
                    });
                let prev_ch = ui.add_enabled(
                    session.current > 0,
                    egui::Button::new("|<- Пред. глава"),
                );
                if prev_ch.clicked() {
                    session.switch_chapter(session.current - 1);
                }
                if ui
                    .add_enabled(
                        session.current + 1 < session.chapters.len(),
                        egui::Button::new("След. глава ->|"),
                    )
                    .clicked()
                {
                    session.switch_chapter(session.current + 1);
                }
                ui.separator();
                for m in Mode::all() {
                    if ui.selectable_label(session.mode == m, m.label()).clicked() {
                        session.mode = m;
                        session.zoom = 1.0;
                        session.pan = Vec2::ZERO;
                        session.jump_to = Some(session.page);
                        lib.settings.default_mode = m as usize;
                        session.dirty = true;
                    }
                }
                ui.separator();
                ComboBox::from_id_salt("reader_fit")
                    .selected_text(session.fit.label())
                    .width(88.0)
                    .show_ui(ui, |ui| {
                        for f in Fit::all() {
                            if ui.selectable_label(session.fit == f, f.label()).clicked() {
                                session.fit = f;
                                session.pan = Vec2::ZERO;
                            }
                        }
                    });
                let rtl_text = if session.rtl { "[RTL ->]" } else { "[<- LTR]" };
                let mut rtl_btn = egui::Button::new(egui::RichText::new(rtl_text).strong());
                if session.rtl {
                    rtl_btn = rtl_btn.fill(egui::Color32::from_rgb(30, 70, 45));
                }
                if ui.add(rtl_btn).clicked() {
                    session.rtl = !session.rtl;
                    lib.settings.rtl = session.rtl;
                    session.dirty = true;
                }
                let ch_key = session
                    .current_chapter()
                    .map(|c| c.key.clone())
                    .unwrap_or_default();
                let marked = session
                    .current_chapter()
                    .map(|c| {
                        lib.entry(&session.manga_id)
                            .map(|e| {
                                e.bookmarks
                                    .iter()
                                    .any(|b| b.chapter == c.key && b.page as usize == session.page)
                            })
                            .unwrap_or(false)
                    })
                    .unwrap_or(false);
                let can_bookmark = !ch_key.is_empty() && session.pages_state.is_ready();
                let star_text = if marked { "[*] Закладка" } else { "[ ] Закладка" };
                let mut star_btn = egui::Button::new(star_text);
                if marked {
                    star_btn = star_btn.fill(egui::Color32::from_rgb(60, 55, 20));
                }
                if ui.add_enabled(can_bookmark, star_btn).clicked() {
                    toggle_bookmark(session, lib);
                }
                if let Some(ch) = session.current_chapter() {
                    if let ChapterSource::Mangadex { id } = &ch.source {
                        let got = if ch.total_pages > 0 {
                            downloads::is_complete(data_dir, id, ch.total_pages)
                        } else {
                            downloads::is_downloaded(data_dir, id)
                        };
                        let busy = dl.running(id);
                        let dl_text = if busy {
                            None
                        } else if got {
                            Some("Скачано")
                        } else {
                            Some("Скачать главу")
                        };
                        let clicked = match dl_text {
                            Some(t) => ui.button(t).clicked(),
                            None => {
                                // live progress of the running download
                                match dl.progress(id) {
                                    Some(pr) if pr.total > 0 => {
                                        ui.monospace(egui::RichText::new(format!(
                                            "[{}] {}/{} стр.",
                                            crate::app::text_bar(pr.frac(), 12),
                                            pr.done,
                                            pr.total
                                        )));
                                    }
                                    _ => {
                                        ui.label("подготовка…");
                                    }
                                }
                                false
                            }
                        };
                        if clicked && !busy && !got {
                            dl.start(id, session.manga_id.clone(), ch.total_pages, data_dir.to_path_buf());
                            (toast)("Загрузка главы началась".to_string(), false);
                        }
                    }
                }
            });
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                let total = session.total_pages();
                if total > 0 {
                    // Five slots, mirrored when RTL: in RTL manga the "next"
                    // page is to the LEFT, so "<" goes forward.
                    #[derive(Clone, Copy)]
                    enum Slot {
                        JumpStart,
                        Prev,
                        Slider,
                        Next,
                        JumpEnd,
                    }
                    let order: [Slot; 5] = if session.rtl {
                        [Slot::JumpEnd, Slot::Next, Slot::Slider, Slot::Prev, Slot::JumpStart]
                    } else {
                        [Slot::JumpStart, Slot::Prev, Slot::Slider, Slot::Next, Slot::JumpEnd]
                    };
                    for slot in order {
                        match slot {
                            Slot::JumpStart => {
                                let (txt, tip) = if session.rtl { (">|", "В начало главы") } else { ("|<", "В начало главы") };
                                if ui.button(txt).on_hover_text(tip).clicked() {
                                    session.page = 0;
                                    session.jump_to = Some(0);
                                    session.pan = Vec2::ZERO;
                                    session.pending_save = true;
                                }
                            }
                            Slot::JumpEnd => {
                                let (txt, tip) = if session.rtl { ("|<", "В конец главы") } else { (">|", "В конец главы") };
                                if ui.button(txt).on_hover_text(tip).clicked() {
                                    session.page = total - 1;
                                    session.jump_to = Some(session.page);
                                    session.pan = Vec2::ZERO;
                                    session.pending_save = true;
                                }
                            }
                            Slot::Prev => {
                                // previous page: visually moves right in RTL
                                let txt = if session.rtl { " > " } else { " < " };
                                if ui.button(txt).on_hover_text("Предыдущая страница").clicked() {
                                    session.turn_page(-1);
                                }
                            }
                            Slot::Next => {
                                let txt = if session.rtl { " < " } else { " > " };
                                if ui.button(txt).on_hover_text("Следующая страница").clicked() {
                                    session.turn_page(1);
                                }
                            }
                            Slot::Slider => {
                                let mut cur = (session.page + 1) as f32;
                                let resp = ui.add(
                                    egui::Slider::new(&mut cur, 1.0..=total as f32)
                                        .show_value(false)
                                        .text(format!("{} / {}", session.page + 1, total)),
                                );
                                if resp.changed() {
                                    session.page = cur as usize - 1;
                                    session.jump_to = Some(session.page);
                                    session.pan = Vec2::ZERO;
                                    session.pending_save = true;
                                }
                            }
                        }
                    }
                } else {
                    ui.spinner();
                    ui.label("Загрузка…");
                }
                if let PagesState::Failed(e) = &session.pages_state {
                    ui.colored_label(egui::Color32::RED, crate::app::truncate(e, 70));
                    if ui.button("Повторить").clicked() {
                        session.pages_state = PagesState::Idle;
                    }
                }
            });
            ui.add_space(4.0);
        });

    // ---- main area ----
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE.fill(egui::Color32::BLACK))
        .show(ctx, |ui| match &session.pages_state {
            PagesState::Ready if !session.pages.is_empty() => {
                prefetch(session, images);
                match session.mode {
                    Mode::Paged => show_paged(session, ui, images),
                    Mode::Strip => show_strip(session, ui, images),
                    Mode::Horizontal => show_horizontal(session, ui, images),
                }
            }
            PagesState::Failed(_) => {
                ui.centered_and_justified(|ui| {
                    ui.label("Не удалось загрузить страницы. Проверьте сеть и нажмите «Повторить».");
                });
            }
            _ => {
                ui.centered_and_justified(|ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(40.0);
                        ui.add(egui::Spinner::new().size(42.0));
                        ui.add_space(10.0);
                        ui.label(if session.chapters_job.is_some() {
                            "Загрузка списка глав…"
                        } else {
                            "Загрузка страниц…"
                        });
                    });
                });
            }
        });

    // ---- progress / persistence ----
    if session.pending_save {
        session.pending_save = false;
        if let Some(ch) = session.current_chapter() {
            let total = session.total_pages() as u32;
            if let Some(e) = lib.entry_mut(&session.manga_id) {
                e.set_progress(&ch.key, session.page as u32, total);
            }
            lib.push_history(HistoryItem {
                manga_id: session.manga_id.clone(),
                title: session.manga_title.clone(),
                chapter: ch.key.clone(),
                chapter_label: ch.label.clone(),
                page: session.page as u32,
                total,
                at: crate::library::now(),
            });
            session.dirty = true;
        }
    }
    ctx.request_repaint_after(std::time::Duration::from_millis(50));
}

fn toggle_bookmark(session: &mut ReaderSession, lib: &mut Library) {
    let (manga_id, ch_key, ch_label, page, total) = {
        let ch = match session.current_chapter() {
            Some(c) => c,
            None => return,
        };
        (
            session.manga_id.clone(),
            ch.key.clone(),
            ch.label.clone(),
            session.page as u32,
            session.total_pages() as u32,
        )
    };
    if let Some(e) = lib.entry_mut(&manga_id) {
        let existing = e
            .bookmarks
            .iter()
            .position(|b| b.chapter == ch_key && b.page == page);
        match existing {
            Some(pos) => {
                e.bookmarks.remove(pos);
            }
            None => {
                e.bookmarks.push(Bookmark {
                    chapter: ch_key,
                    chapter_label: ch_label,
                    page,
                    total,
                    at: crate::library::now(),
                });
            }
        }
    }
    session.pending_save = true;
}

fn handle_input(session: &mut ReaderSession, ctx: &egui::Context) {
    ctx.input(|i| {
        if i.key_pressed(Key::Escape) {
            session.start_hint = Some("__close__".into());
        }
        if !session.pages_state.is_ready() {
            return;
        }
        // wheel zoom / page turn
        let delta = i.raw_scroll_delta;
        if i.modifiers.ctrl {
            if delta.y != 0.0 {
                session.zoom = (session.zoom * (1.0 + delta.y * 0.002)).clamp(0.2, 8.0);
            }
        } else if delta.y != 0.0 {
            session.wheel_acc += delta.y;
            let threshold = 60.0;
            if session.wheel_acc <= -threshold {
                session.wheel_acc = 0.0;
                session.turn_page(1);
            } else if session.wheel_acc >= threshold {
                session.wheel_acc = 0.0;
                session.turn_page(-1);
            }
        }
        if i.key_pressed(Key::ArrowRight) {
            session.turn_page(if session.rtl { -1 } else { 1 });
        } else if i.key_pressed(Key::ArrowLeft) {
            session.turn_page(if session.rtl { 1 } else { -1 });
        }
        if i.key_pressed(Key::Space) {
            session.turn_page(if i.modifiers.shift { -1 } else { 1 });
        }
        if i.key_pressed(Key::PageDown) {
            session.turn_page(1);
        }
        if i.key_pressed(Key::PageUp) {
            session.turn_page(-1);
        }
        if i.key_pressed(Key::Home) {
            session.page = 0;
            session.jump_to = Some(0);
            session.pending_save = true;
        }
        if i.key_pressed(Key::End) {
            session.page = session.total_pages().saturating_sub(1);
            session.jump_to = Some(session.page);
            session.pending_save = true;
        }
        if i.key_pressed(Key::Plus) || i.key_pressed(Key::Equals) {
            session.zoom = (session.zoom * 1.25).clamp(0.2, 8.0);
        }
        if i.key_pressed(Key::Minus) {
            session.zoom = (session.zoom / 1.25).clamp(0.2, 8.0);
        }
        if i.key_pressed(Key::Num0) {
            session.zoom = 1.0;
            session.pan = Vec2::ZERO;
        }
    });
}

fn prefetch(session: &mut ReaderSession, images: &mut ImageCache) {
    if !session.pages_state.is_ready() {
        return;
    }
    let p = session.page;
    let n = session.pages.len();
    let idx: Vec<usize> = [
        Some(p),
        p.checked_sub(1),
        p.checked_add(1),
        p.checked_add(2),
        p.checked_add(3),
    ]
    .into_iter()
    .flatten()
    .filter(|&i| i < n)
    .collect();
    for i in idx {
        let src = session.pages[i].clone();
        let key = src_key(&src);
        images.request(&key, src);
    }
}

fn src_key(src: &ImgSource) -> String {
    match src {
        ImgSource::Url(u) => format!("url:{u}"),
        ImgSource::UrlDisk { url, .. } => format!("url:{url}"),
        ImgSource::Page(p) => format!("page:{}", p.key()),
    }
}

fn truncate_err(s: &str) -> String {
    if s.chars().count() <= 90 {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(90).collect::<String>())
    }
}

fn fit_scale(fit: Fit, avail: Vec2, ts: Vec2) -> f32 {
    if ts.x <= 0.0 || ts.y <= 0.0 {
        return 1.0;
    }
    match fit {
        Fit::Best => (avail.x / ts.x).min(avail.y / ts.y),
        Fit::Width => avail.x / ts.x,
        Fit::Height => avail.y / ts.y,
        Fit::Original => 1.0,
    }
}

fn draw_texture(ui: &mut Ui, tex: &TextureHandle, rect: egui::Rect) {
    ui.painter().image(
        tex.id(),
        rect,
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        egui::Color32::WHITE,
    );
}
use eframe::egui::Ui;

// ---------- paged mode ----------

fn show_paged(session: &mut ReaderSession, ui: &mut Ui, images: &mut ImageCache) {
    let avail = ui.available_size();
    let avail_rect = ui.available_rect_before_wrap();
    let src = session.pages[session.page].clone();
    let key = src_key(&src);
    let tex = images.get(&key).cloned();

    if let Some(tex) = tex.as_ref() {
        let ts = tex.size_vec2();
        let scale = fit_scale(session.fit, avail, ts) * session.zoom;
        let size = (ts * scale).max(Vec2::splat(1.0));
        // clamp pan so the image stays reachable
        let max_pan = ((size - avail) / 2.0).max(Vec2::ZERO);
        session.pan = session.pan.clamp(-max_pan, max_pan);
        let center = avail_rect.center() + session.pan;
        let rect = egui::Rect::from_center_size(center, size);
        let resp = ui.allocate_rect(avail_rect, Sense::click_and_drag());
        draw_texture(ui, tex, rect);

        if resp.dragged() {
            session.pan += resp.drag_delta();
            session.pan = session.pan.clamp(-max_pan, max_pan);
        }
        if resp.double_clicked() {
            session.zoom = 1.0;
            session.pan = Vec2::ZERO;
        } else if resp.clicked() {
            if let Some(pos) = resp.interact_pointer_pos() {
                let rel = (pos.x - avail_rect.left()) / avail_rect.width();
                let (next_frac, prev_frac) = if session.rtl { (0.35, 0.65) } else { (0.65, 0.35) };
                if rel < next_frac {
                    session.turn_page(if session.rtl { 1 } else { -1 });
                } else if rel > prev_frac {
                    session.turn_page(if session.rtl { -1 } else { 1 });
                } else {
                    // middle zone: toggle zoom
                    if session.zoom > 1.01 {
                        session.zoom = 1.0;
                        session.pan = Vec2::ZERO;
                    } else {
                        session.zoom = 2.0;
                    }
                }
            }
        }
    } else {
        let failed = images.failed(&key).map(|s| s.to_string());
        if failed.is_none() {
            images.request(&key, src);
        }
        let resp = ui.allocate_rect(avail_rect, Sense::click());
        let c = avail_rect.center();
        if let Some(msg) = failed {
            ui.painter().text(
                c,
                egui::Align2::CENTER_CENTER,
                format!("{}\n\n(клик — повторить)", truncate_err(&msg)),
                egui::FontId::default(),
                egui::Color32::RED,
            );
            if resp.clicked() {
                images.invalidate(&key);
            }
        } else {
            ui.painter().circle_filled(c, 18.0, egui::Color32::from_rgb(90, 90, 120));
        }
    }
}

// ---------- strip mode ----------

fn show_strip(session: &mut ReaderSession, ui: &mut Ui, images: &mut ImageCache) {
    let avail = ui.available_size();
    let rects: RefCell<Vec<egui::Rect>> = RefCell::new(Vec::new());
    let mut out = egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .drag_to_scroll(true)
        .show(ui, |ui| {
            ui.style_mut().spacing.item_spacing.y = 4.0;
            for (i, src) in session.pages.iter().enumerate() {
                let key = src_key(src);
                let w = avail.x * session.zoom;
                let h = if let Some(tex) = images.get(&key) {
                    let ts = tex.size_vec2();
                    ts.y * (w / ts.x.max(1.0))
                } else {
                    w * 1.4 // placeholder aspect until loaded
                };
                let size = Vec2::new(w, h.max(1.0));
                let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
                if let Some(tex) = images.get(&key) {
                    draw_texture(ui, tex, rect);
                } else {
                    images.request(&key, src.clone());
                    ui.painter()
                        .rect_filled(rect, 4.0, egui::Color32::from_rgb(18, 18, 24));
                    if let Some(_err) = images.failed(&key) {
                        ui.painter().text(
                            rect.center(),
                            egui::Align2::CENTER_CENTER,
                            "ошибка загрузки\n(клик — повторить)",
                            egui::FontId::default(),
                            egui::Color32::RED,
                        );
                        if resp.clicked() {
                            images.invalidate(&key);
                        }
                    } else {
                        ui.painter().text(
                            rect.center(),
                            egui::Align2::CENTER_CENTER,
                            "…",
                            egui::FontId::proportional(28.0),
                            egui::Color32::GRAY,
                        );
                    }
                }
                rects.borrow_mut().push(rect);
            }
        });
    let rects = rects.into_inner();
    if !rects.is_empty() {
        let view = out.inner_rect;
        let center_y = view.center().y;
        if let Some(i) = rects
            .iter()
            .position(|r| center_y >= r.top() && center_y < r.bottom())
        {
            session.page = i;
        }
        if let Some(j) = session.jump_to.take() {
            if let Some(r) = rects.get(j) {
                out.state.offset.y = r.top() - rects[0].top() + view.top();
            }
        }
    }
    if ui.input(|i| i.pointer.button_double_clicked(egui::PointerButton::Primary)) {
        session.zoom = 1.0;
    }
    if ui.input(|i| i.modifiers.ctrl) && ui.input(|i| i.raw_scroll_delta.y != 0.0) {
        let dy = ui.input(|i| i.raw_scroll_delta.y);
        session.zoom = (session.zoom * (1.0 + dy * 0.002)).clamp(0.2, 8.0);
    }
}

// ---------- horizontal mode ----------

fn show_horizontal(session: &mut ReaderSession, ui: &mut Ui, images: &mut ImageCache) {
    let avail = ui.available_size();
    let rects: RefCell<Vec<egui::Rect>> = RefCell::new(Vec::new());
    let mut out = egui::ScrollArea::horizontal()
        .auto_shrink([false, false])
        .drag_to_scroll(true)
        .show(ui, |ui| {
            ui.style_mut().spacing.item_spacing.x = 4.0;
            if session.rtl {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    draw_horizontal_pages(session, ui, images, &rects, avail);
                });
            } else {
                draw_horizontal_pages(session, ui, images, &rects, avail);
            }
        });
    let rects = rects.into_inner();
    if !rects.is_empty() {
        let view = out.inner_rect;
        let center_x = view.center().x;
        if let Some(i) = rects
            .iter()
            .position(|r| center_x >= r.left() && center_x < r.right())
        {
            session.page = i;
        }
        if let Some(j) = session.jump_to.take() {
            if let Some(r) = rects.get(j) {
                out.state.offset.x = r.left() - rects[0].left();
            }
        }
    }
    if ui.input(|i| i.pointer.button_double_clicked(egui::PointerButton::Primary)) {
        session.zoom = 1.0;
    }
    if ui.input(|i| i.modifiers.ctrl) && ui.input(|i| i.raw_scroll_delta.y != 0.0) {
        let dy = ui.input(|i| i.raw_scroll_delta.y);
        session.zoom = (session.zoom * (1.0 + dy * 0.002)).clamp(0.2, 8.0);
    }
}

fn draw_horizontal_pages(
    session: &mut ReaderSession,
    ui: &mut Ui,
    images: &mut ImageCache,
    rects: &RefCell<Vec<egui::Rect>>,
    avail: Vec2,
) {
    {
            for (i, src) in session.pages.iter().enumerate() {
                let key = src_key(src);
                let h = avail.y * session.zoom;
                let w = if let Some(tex) = images.get(&key) {
                    let ts = tex.size_vec2();
                    ts.x * (h / ts.y.max(1.0))
                } else {
                    h * 0.75
                };
                let size = Vec2::new(w.max(1.0), h);
                let (rect, _) = ui.allocate_exact_size(size, Sense::click());
                if let Some(tex) = images.get(&key) {
                    draw_texture(ui, tex, rect);
                } else {
                    images.request(&key, src.clone());
                    ui.painter()
                        .rect_filled(rect, 4.0, egui::Color32::from_rgb(18, 18, 24));
                }
                rects.borrow_mut().push(rect);
            }
    }
}
