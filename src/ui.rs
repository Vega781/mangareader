//! All screens except the reader: library, search, detail, history, settings.
use eframe::egui::{self, Sense};

use crate::app::{cover_task, truncate, App, Screen};
use crate::images::ImgSource;
use crate::jobs::Job;
use crate::library::{CoverSource, HistoryItem};
use crate::mangadex::{self, MangaSummary};
use crate::reader::{self, ChapterSource};

impl App {
    pub fn show_main_ui(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("tabs")
            .frame(egui::Frame::default().fill(egui::Color32::from_rgb(24, 24, 30)))
            .show(ctx, |ui| {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.add_space(8.0);
                    for (screen, label) in [
                        (Screen::Library, "📚 Библиотека"),
                        (Screen::Search, "🔍 Поиск MangaDex"),
                        (Screen::History, "🕘 История"),
                        (Screen::Settings, "⚙ Настройки"),
                    ] {
                        if ui
                            .selectable_label(self.screen == screen.clone(), label)
                            .clicked()
                        {
                            self.screen = screen;
                        }
                        ui.add_space(4.0);
                    }
                });
                ui.add_space(4.0);
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            match self.screen.clone() {
                Screen::Library => self.show_library(ui),
                Screen::Search => self.show_search(ui),
                Screen::Detail(id) => self.show_detail(ui, &id),
                Screen::History => self.show_history(ui),
                Screen::Settings => self.show_settings(ui),
            }
        });
    }

    // ---------- cover helper ----------

    fn cover_pair(&self, id: &str, cover: Option<&CoverSource>) -> Option<(String, ImgSource)> {
        match cover {
            Some(CoverSource::MdUrl(url)) => {
                let safe = id.replace([':', '/'], "_");
                Some((
                    format!("cover:{safe}"),
                    ImgSource::UrlDisk {
                        url: url.clone(),
                        local: self
                            .data_dir
                            .join("covers")
                            .join(format!("{safe}.jpg")),
                    },
                ))
            }
            Some(CoverSource::LocalPage(p)) => {
                let safe = id.replace([':', '/'], "_");
                Some((format!("cover:{safe}"), ImgSource::Page(p.clone())))
            }
            None => None,
        }
    }

    fn cover_from_summary(&self, s: &MangaSummary) -> Option<(String, ImgSource)> {
        self.cover_pair(&format!("md:{}", s.id), Some(&CoverSource::MdUrl(s.cover_url.clone()?)))
    }

    fn draw_cover(
        &mut self,
        ui: &mut egui::Ui,
        pair: Option<(String, ImgSource)>,
        size: egui::Vec2,
    ) -> egui::Response {
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
        match pair {
            Some((key, src)) => {
                if let Some(tex) = self.images.get(&key) {
                    ui.painter().image(
                        tex.id(),
                        rect,
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        egui::Color32::WHITE,
                    );
                } else {
                    self.images.request(&key, src);
                    ui.painter()
                        .rect_filled(rect, 6.0, egui::Color32::from_rgb(30, 30, 38));
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        "…",
                        egui::FontId::proportional(28.0),
                        egui::Color32::GRAY,
                    );
                }
            }
            None => {
                ui.painter()
                    .rect_filled(rect, 6.0, egui::Color32::from_rgb(30, 30, 38));
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "нет обложки",
                    egui::FontId::default(),
                    egui::Color32::GRAY,
                );
            }
        }
        resp
    }

    // ---------- library ----------

    fn show_library(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            ui.heading("Библиотека");
            if ui.button("🔍 Поиск MangaDex").clicked() {
                self.screen = Screen::Search;
            }
            if ui.button("📂 Открыть CBZ/папку…").clicked() {
                self.browse_local();
            }
            ui.label("или путь:");
            let resp = ui.add(
                egui::TextEdit::singleline(&mut self.local_path_input)
                    .desired_width(240.0)
                    .hint_text("/путь/к/манге или файл.cbz"),
            );
            if ui.button("Добавить").clicked() || resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                let p = self.local_path_input.clone();
                self.local_path_input.clear();
                self.add_local(&p);
            }
        });
        ui.add_space(8.0);

        if self.lib.entries.is_empty() {
            ui.vertical_centered(|ui| {
                ui.add_space(60.0);
                ui.label(egui::RichText::new("Библиотека пуста").size(20.0));
                ui.add_space(8.0);
                ui.label("Найдите мангу через поиск MangaDex или добавьте локальные файлы (CBZ, папки с картинками).");
            });
            return;
        }

        let ids: Vec<String> = self.lib.entries.iter().map(|e| e.id.clone()).collect();
        let card_w = 170.0_f32;
        let card_h = 300.0_f32;
        let cols = ((ui.available_width() / (card_w + 14.0)).floor() as usize).max(1);
        let mut open_detail: Option<String> = None;
        let mut open_reader: Option<(String, Option<String>)> = None;
        let mut to_remove: Option<String> = None;

        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Grid::new("lib_grid")
                .spacing([14.0, 18.0])
                .show(ui, |ui| {
                    for (i, id) in ids.iter().enumerate() {
                        if i % cols == 0 && i > 0 {
                            ui.end_row();
                        }
                        let entry = match self.lib.entry(id).cloned() {
                            Some(e) => e,
                            None => continue,
                        };
                        let mut inner_clicked = false;
                        let pair = cover_task(&entry, &self.data_dir);
                        let cover_resp = self.draw_cover(ui, pair, egui::vec2(card_w, card_w * 1.42));
                        ui.vertical(|ui| {
                            ui.set_width(card_w);
                            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                            ui.label(egui::RichText::new(truncate(&entry.title, 24)).strong());
                            if let Some(p) = entry.progress_percent() {
                                ui.add(egui::ProgressBar::new(p / 100.0).show_percentage().desired_width(card_w));
                            }
                            ui.horizontal(|ui| {
                                if ui.small_button("Читать").clicked() {
                                    inner_clicked = true;
                                    let hint = entry.last_chapter.clone();
                                    open_reader = Some((entry.id.clone(), hint));
                                }
                                if ui.small_button("✕").clicked() {
                                    inner_clicked = true;
                                    to_remove = Some(entry.id.clone());
                                }
                            });
                        });
                        if cover_resp.clicked() && !inner_clicked {
                            open_detail = Some(entry.id.clone());
                        }
                    }
                });
        });

        let _ = card_h;
        if let Some(id) = open_detail {
            self.open_detail(id, None);
        }
        if let Some((id, hint)) = open_reader {
            self.open_reader(id, hint);
        }
        if let Some(id) = to_remove.take() {
            self.lib.remove(&id);
            self.save_needed = true;
        }
    }

    // ---------- search ----------

    fn run_search_job(&mut self, query: String, offset: u32, append: bool) {
        let job = Job::spawn(move || {
            if query.trim().is_empty() {
                mangadex::popular(offset)
            } else {
                mangadex::search(&query, offset)
            }
        });
        self.search.job = Some(job);
        self.search.append = append;
        self.search.loading = true;
        self.search.searched_once = true;
    }

    fn show_search(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            ui.heading("Поиск MangaDex");
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            let resp = ui.add(
                egui::TextEdit::singleline(&mut self.search.query)
                    .desired_width(360.0)
                    .hint_text("Название манги (пусто = популярное)"),
            );
            if ui.button("🔍 Найти").clicked()
                || resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))
            {
                self.search.results.clear();
                let q = self.search.query.clone();
                self.run_search_job(q, 0, false);
            }
            if self.search.loading {
                ui.spinner();
            }
        });
        ui.add_space(8.0);

        // poll job
        if let Some(job) = self.search.job.as_mut() {
            if let Some(res) = job.poll() {
                self.search.job = None;
                self.search.loading = false;
                match res {
                    Ok((mut list, total)) => {
                        if self.search.append {
                            self.search.results.append(&mut list);
                        } else {
                            self.search.results = list;
                        }
                        self.search.total = total;
                    }
                    Err(e) => self.toast(format!("Ошибка поиска: {e}"), true),
                }
            }
        }

        // auto popular on first visit
        if self.search.results.is_empty()
            && self.search.job.is_none()
            && !self.search.loading
            && !self.search.searched_once
        {
            self.run_search_job(String::new(), 0, false);
        }

        let cards: Vec<MangaSummary> = self.search.results.clone();
        let mut open_detail: Option<(String, MangaSummary)> = None;
        let mut toggled: Option<MangaSummary> = None;
        let mut load_more = false;

        egui::ScrollArea::vertical().show(ui, |ui| {
            let card_w = 170.0_f32;
            let cols = ((ui.available_width() / (card_w + 14.0)).floor() as usize).max(1);
            egui::Grid::new("search_grid")
                .spacing([14.0, 18.0])
                .show(ui, |ui| {
                    for (i, s) in cards.iter().enumerate() {
                        if i % cols == 0 && i > 0 {
                            ui.end_row();
                        }
                        let id = format!("md:{}", s.id);
                        let mut inner_clicked = false;
                        let pair = self.cover_from_summary(s);
                        let cover_resp = self.draw_cover(ui, pair, egui::vec2(card_w, card_w * 1.42));
                        ui.vertical(|ui| {
                            ui.set_width(card_w);
                            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                            ui.label(egui::RichText::new(truncate(&s.title, 24)).strong());
                            ui.horizontal(|ui| {
                                let in_lib = self.lib.contains(&id);
                                if ui.small_button(if in_lib { "✓ В библиотеке" } else { "+ Добавить" }).clicked() {
                                    inner_clicked = true;
                                    toggled = Some(s.clone());
                                }
                            });
                        });
                        if cover_resp.clicked() && !inner_clicked {
                            open_detail = Some((id, s.clone()));
                        }
                    }
                });
            ui.add_space(10.0);
            if self.search.results.len() < self.search.total as usize && !self.search.loading {
                if ui.button("Ещё результаты…").clicked() {
                    load_more = true;
                }
            }
        });

        if let Some(s) = toggled {
            self.toggle_library(&s);
        }
        if let Some((id, s)) = open_detail {
            self.open_detail(id, Some(s));
        }
        if load_more {
            let q = self.search.query.clone();
            let offset = self.search.results.len() as u32;
            self.run_search_job(q, offset, true);
        }
    }

    fn show_detail(&mut self, ui: &mut egui::Ui, id: &str) {
        if !self.detail.as_ref().map(|d| d.manga_id == id).unwrap_or(false) {
            self.screen = Screen::Library;
            return;
        }

        // poll jobs (scoped borrow)
        let (mut summary, mut chapters, mut ch_loading, mut job_errors) = {
            let ds = self.detail.as_mut().unwrap();
            let mut errs: Vec<String> = Vec::new();
            if let Some(job) = ds.sum_job.as_mut() {
                if let Some(res) = job.poll() {
                    ds.sum_job = None;
                    match res {
                        Ok(s) => ds.summary = Some(s),
                        Err(e) => errs.push(e),
                    }
                }
            }
            if let Some(job) = ds.ch_job.as_mut() {
                if let Some(res) = job.poll() {
                    ds.ch_job = None;
                    match res {
                        Ok(chs) => ds.chapters = chs,
                        Err(e) => errs.push(e),
                    }
                }
            }
            (ds.summary.clone(), ds.chapters.clone(), ds.ch_job.is_some(), errs)
        };
        for e in job_errors {
            self.toast(e, true);
        }

        // update library entry info if present
        let summary_title = summary.as_ref().map(|s| s.title.clone()).unwrap_or_default();
        let summary_desc = summary.as_ref().map(|s| s.description.clone()).unwrap_or_default();
        let summary_status = summary.as_ref().map(|s| s.status.clone()).unwrap_or_default();
        let summary_cover = summary.as_ref().and_then(|s| s.cover_url.clone());
        let has_summary = summary.is_some();
        if !summary_title.is_empty() || !summary_desc.is_empty() || summary_cover.is_some() {
            if let Some(e) = self.lib.entry_mut(id) {
                if !summary_title.is_empty() {
                    e.title = summary_title.clone();
                }
                if !summary_desc.is_empty() {
                    e.description = summary_desc.clone();
                }
                if e.cover.is_none() {
                    if let Some(u) = &summary_cover {
                        e.cover = Some(CoverSource::MdUrl(u.clone()));
                    }
                }
            }
        }

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            if crate::icons::button_with_text(ui, crate::icons::Icon::Prev, "Назад", false).clicked() {
                self.screen = Screen::Library;
            }
            ui.heading(truncate(&summary_title, 50));
        });
        ui.add_space(8.0);

        let entry_present = self.lib.contains(id);
        let entry = self.lib.entry(id).cloned();
        let mut read_clicked = false;
        let mut continue_clicked = false;
        let mut toggle_lib = false;
        let mut open_reader_ch: Option<String> = None;
        let mut remove_bookmark: Option<usize> = None;

        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.horizontal(|ui| {
                let cover_pair = entry
                    .as_ref()
                    .and_then(|e| cover_task(e, &self.data_dir))
                    .or_else(|| summary.as_ref().and_then(|s| self.cover_from_summary(s)));
                let _ = self.draw_cover(ui, cover_pair, egui::vec2(200.0, 285.0));
                ui.add_space(12.0);
                ui.vertical(|ui| {
                    ui.set_width(ui.available_width() - 12.0);
                    if !summary_status.is_empty() {
                        let st = match summary_status.as_str() {
                            "ongoing" => "Выходит",
                            "completed" => "Завершена",
                            "hiatus" => "Перерыв",
                            "cancelled" => "Отменена",
                            _ => "Статус неизвестен",
                        };
                        ui.label(egui::RichText::new(format!("Статус: {st}")).weak());
                    }
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        if crate::icons::button_with_text(ui, crate::icons::Icon::Next, "Читать", false).clicked() {
                            read_clicked = true;
                        }
                        if entry_present
                            && crate::icons::button_with_text(ui, crate::icons::Icon::Next, "Продолжить", false)
                                .clicked()
                        {
                            continue_clicked = true;
                        }
                        if has_summary {
                            let label = if entry_present { "\u{2212} Убрать из библиотеки" } else { "+ В библиотеку" };
                            if ui.button(label).clicked() {
                                toggle_lib = true;
                            }
                        }
                    });
                    if let Some(e) = &entry {
                        if !e.finished.is_empty() {
                            ui.label(egui::RichText::new(format!("Прочитано глав: {}", e.finished.len())).weak());
                        }
                        if !e.downloaded.is_empty() {
                            ui.label(egui::RichText::new(format!("Скачано глав: {}", e.downloaded.len())).weak());
                        }
                    }
                    ui.add_space(6.0);
                    if !summary_desc.is_empty() {
                        egui::ScrollArea::vertical()
                            .id_salt("desc_scroll")
                            .max_height(130.0)
                            .show(ui, |ui| {
                                ui.label(egui::RichText::new(truncate(&summary_desc, 2000)).weak());
                            });
                    }
                });
            });
            ui.add_space(10.0);

            if let Some(e) = &entry {
                if !e.bookmarks.is_empty() {
                    ui.heading("\u{2605} Закладки");
                    ui.add_space(4.0);
                    for (i, b) in e.bookmarks.iter().enumerate() {
                        ui.horizontal(|ui| {
                            ui.label(format!(
                                "\u{2605} {} \u{2014} стр. {}{}",
                                truncate(&b.chapter_label, 40),
                                b.page + 1,
                                if b.total > 0 { format!("/{}", b.total) } else { String::new() }
                            ));
                            if ui.small_button("Открыть").clicked() {
                                open_reader_ch = Some(b.chapter.clone());
                            }
                            if ui.small_button("\u{2715}").clicked() {
                                remove_bookmark = Some(i);
                            }
                        });
                    }
                    ui.add_space(8.0);
                }
            }

            // download-all queue controls (MangaDex only)
            if chapters.iter().any(|c| matches!(c.source, ChapterSource::Mangadex { .. })) {
                let entry = self.lib.entry(id).cloned();
                let missing: Vec<(String, String)> = chapters
                    .iter()
                    .filter_map(|c| match &c.source {
                        ChapterSource::Mangadex { id: cid } => {
                            let downloaded = entry
                                .as_ref()
                                .map(|e| e.downloaded.contains(cid))
                                .unwrap_or(false)
                                || crate::downloads::is_downloaded(&self.data_dir, cid);
                            if downloaded || self.downloads.running(cid) || self.downloads.queued(cid) {
                                None
                            } else {
                                Some((cid.clone(), id.to_string()))
                            }
                        }
                        _ => None,
                    })
                    .collect();
                ui.horizontal(|ui| {
                    let active = self.downloads.active();
                    let queued = self.downloads.pending();
                    if !missing.is_empty() {
                        if ui.button(format!("Скачать всю мангу ({})", missing.len())).clicked() {
                            for (cid, mid) in &missing {
                                self.downloads.enqueue(cid.clone(), mid.clone());
                            }
                            self.toast(format!("В очередь добавлено глав: {}", missing.len()), false);
                        }
                    }
                    if active + queued > 0 {
                        ui.spinner();
                        ui.label(format!("Скачивание: {} сейчас, {} в очереди", active, queued));
                        if ui.button("Остановить").clicked() {
                            self.downloads.clear_queue();
                            self.toast("Очередь очищена (текущая глава докачается)".into(), false);
                        }
                    }
                });
                ui.add_space(4.0);
            }

            ui.heading(format!("Главы ({})", chapters.len()));
            ui.add_space(4.0);
            if ch_loading {
                ui.spinner();
            } else if chapters.is_empty() {
                ui.label("Главы не найдены (проверьте языки в настройках).");
            }
            egui::ScrollArea::vertical().show(ui, |ui| {
                let entry = self.lib.entry(id).cloned();
                for ch in &chapters {
                    ui.horizontal(|ui| {
                        let prog = entry
                            .as_ref()
                            .and_then(|e| e.progress.get(&ch.key).copied())
                            .unwrap_or(0);
                        let finished = entry
                            .as_ref()
                            .map(|e| e.finished.contains(&ch.key))
                            .unwrap_or(false);
                        {
                            let (dot_rect, _) =
                                ui.allocate_exact_size(egui::vec2(14.0, 20.0), Sense::hover());
                            let cc = dot_rect.center();
                            let p = ui.painter();
                            if finished {
                                p.circle_filled(cc, 4.5, egui::Color32::from_rgb(90, 200, 120));
                            } else if prog > 0 {
                                p.circle_filled(cc, 4.5, egui::Color32::from_rgb(220, 180, 80));
                                p.circle_filled(
                                    cc + egui::vec2(1.5, 1.5),
                                    3.0,
                                    egui::Color32::from_rgb(18, 18, 24),
                                );
                            } else {
                                p.circle_filled(cc, 3.0, egui::Color32::from_rgb(90, 90, 100));
                            }
                        }
                        ui.label(truncate(&ch.label, 52));
                        if ch.total_pages > 0 {
                            ui.label(egui::RichText::new(format!("{} стр.", ch.total_pages)).weak());
                        }
                        if let ChapterSource::Mangadex { id: cid } = &ch.source {
                            let got = crate::downloads::is_downloaded(&self.data_dir, cid);
                            let busy = self.downloads.running(cid);
                            let queued = self.downloads.queued(cid);
                            if busy {
                                ui.spinner();
                            } else if queued {
                                ui.label(egui::RichText::new("в очереди").weak());
                            } else if got {
                                crate::icons::button_sized(ui, crate::icons::Icon::Check, "Глава скачана, доступна офлайн", 22.0);
                            } else if crate::icons::button_sized(ui, crate::icons::Icon::Download, "Скачать главу", 22.0).clicked() {
                                self.downloads.start(cid, id.to_string(), self.data_dir.clone());
                                self.toast("Загрузка главы началась".into(), false);
                            }
                        }
                        if ui.small_button("Читать").clicked() {
                            open_reader_ch = Some(ch.key.clone());
                        }
                    });
                }
            });
        });

        // store polled state back
        if let Some(ds) = self.detail.as_mut() {
            ds.summary = summary.clone();
            ds.chapters = chapters.clone();
        }

        if read_clicked {
            self.open_reader(id.to_string(), None);
        }
        if continue_clicked {
            let hint = entry.as_ref().and_then(|e| e.last_chapter.clone());
            self.open_reader(id.to_string(), hint);
        }
        if toggle_lib {
            if let Some(s) = summary.clone() {
                self.toggle_library(&s);
            }
        }
        if let Some(ch) = open_reader_ch {
            self.open_reader(id.to_string(), Some(ch));
        }
        if let Some(i) = remove_bookmark {
            if let Some(e) = self.lib.entry_mut(id) {
                if i < e.bookmarks.len() {
                    e.bookmarks.remove(i);
                    self.save_needed = true;
                }
            }
        }
    }

    // ---------- history ----------

    fn show_history(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            ui.heading("История чтения");
            if !self.lib.history.is_empty() && ui.button("Очистить").clicked() {
                self.lib.history.clear();
                self.save_needed = true;
            }
        });
        ui.add_space(8.0);

        let items: Vec<HistoryItem> = self.lib.history.clone();
        let mut open: Option<(String, Option<String>)> = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            if items.is_empty() {
                ui.label("Пока пусто.");
            }
            for h in items {
                ui.horizontal(|ui| {
                    let ago = crate::library::now().saturating_sub(h.at);
                    let ago_s = if ago < 3600 {
                        format!("{} мин назад", ago / 60)
                    } else if ago < 86400 {
                        format!("{} ч назад", ago / 3600)
                    } else {
                        format!("{} дн назад", ago / 86400)
                    };
                    ui.label(egui::RichText::new(ago_s).weak().monospace());
                    ui.label(egui::RichText::new(truncate(&h.title, 30)).strong());
                    ui.label(truncate(&h.chapter_label, 40));
                    ui.label(egui::RichText::new(format!("стр. {}/{}", h.page + 1, h.total)).weak());
                    if ui.small_button("Открыть").clicked() {
                        open = Some((h.manga_id.clone(), Some(h.chapter.clone())));
                    }
                });
            }
        });
        if let Some((id, hint)) = open {
            self.open_reader(id, hint);
        }
    }

    // ---------- settings ----------

    fn show_settings(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            ui.heading("Настройки");
        });
        ui.add_space(8.0);

        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.add_space(4.0);

            ui.group(|ui| {
                ui.label(egui::RichText::new("Читалка").strong());
                ui.add_space(4.0);
                let mut mode = self.lib.settings.default_mode;
                ui.horizontal(|ui| {
                    ui.label("Режим по умолчанию:");
                    for m in reader::Mode::all() {
                        if ui
                            .selectable_label(mode == m as usize, m.label())
                            .clicked()
                        {
                            mode = m as usize;
                            self.lib.settings.default_mode = mode;
                            self.save_needed = true;
                        }
                    }
                });
                ui.checkbox(&mut self.lib.settings.rtl, "Читать справа налево (RTL, как в японской манге)")
                    .changed();
                self.save_needed = true;
                ui.label("В читалке: колесо/клики — листание, Ctrl+колесо — зум, средняя зона — зум 2×, Esc — выход.");
            });

            ui.add_space(8.0);
            ui.group(|ui| {
                ui.label(egui::RichText::new("MangaDex: языки глав").strong());
                ui.add_space(4.0);
                for (code, name) in LANGS {
                    let mut on = self.lib.settings.languages.iter().any(|l| l == code);
                    if ui.checkbox(&mut on, format!("{name} ({code})")).changed() {
                        if on {
                            if !self.lib.settings.languages.iter().any(|l| l == code) {
                                self.lib.settings.languages.push(code.to_string());
                            }
                        } else {
                            self.lib.settings.languages.retain(|l| l != code);
                        }
                        self.save_needed = true;
                    }
                }
            });

            ui.add_space(8.0);
            ui.group(|ui| {
                ui.label(egui::RichText::new("MangaDex: рейтинг контента").strong());
                ui.add_space(4.0);
                for (code, name) in [("safe", "Безопасный"), ("suggestive", "Пикантный"), ("erotica", "Эротика")] {
                    let mut on = self.lib.settings.content_ratings.iter().any(|l| l == code);
                    if ui.checkbox(&mut on, name).changed() {
                        if on {
                            self.lib.settings.content_ratings.push(code.to_string());
                        } else {
                            self.lib.settings.content_ratings.retain(|l| l != code);
                        }
                        self.save_needed = true;
                    }
                }
            });

            ui.add_space(8.0);
            ui.group(|ui| {
                ui.label(egui::RichText::new("Кэш и данные").strong());
                ui.add_space(4.0);
                ui.label(format!("Данные: {}", self.data_dir.display()));
                ui.label(format!(
                    "Текстур в памяти: {}  •  Загрузок в процессе: {}",
                    self.images.len(),
                    self.downloads.count()
                ));
                ui.horizontal(|ui| {
                    if ui.button("Очистить кэш картинок (память)").clicked() {
                        self.images.clear();
                        self.toast("Кэш картинок очищен".into(), false);
                    }
                    if ui.button("Удалить все скачанные главы").clicked() {
                        let dir = self.data_dir.join("downloads");
                        if std::fs::remove_dir_all(&dir).is_ok() {
                            for e in self.lib.entries.iter_mut() {
                                e.downloaded.clear();
                            }
                            self.save_needed = true;
                            self.toast("Скачанные главы удалены".into(), false);
                        }
                    }
                });
            });
        });
    }
}

const LANGS: &[(&str, &str)] = &[
    ("en", "Английский"),
    ("ru", "Русский"),
    ("ja", "Японский"),
    ("uk", "Украинский"),
    ("es", "Испанский"),
    ("fr", "Французский"),
    ("de", "Немецкий"),
    ("pt", "Португальский"),
    ("it", "Итальянский"),
    ("ko", "Корейский"),
    ("zh", "Китайский"),
    ("tr", "Турецкий"),
    ("pl", "Польский"),
];
