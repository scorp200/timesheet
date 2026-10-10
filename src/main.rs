// Hide the console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod storage;

use std::{collections::BTreeMap, path::PathBuf};

use chrono::{Datelike, Duration, Local, NaiveDate, Timelike};
use eframe::egui::{
    self, Align2, Color32, CornerRadius, CursorIcon, FontId, Id, LayerId, Order, Pos2, Rect, Sense,
    Stroke, StrokeKind, Vec2,
};
use serde::{Deserialize, Serialize};
use storage::{Settings, Theme};

const SLOTS_PER_DAY: usize = 24 * 4;
const ROW_H: f32 = 18.0;
const HEADER_H: f32 = 40.0;
const GUTTER_W: f32 = 80.0;
const WEEK_TARGET_HOURS: f32 = 40.0;
/// Key data was stored under in eframe storage by earlier versions; read once for migration.
const LEGACY_STORAGE_KEY: &str = "timesheet";

/// Pastel defaults for new projects, ordered so neighbours contrast.
const PALETTE: [[u8; 3]; 12] = [
    [0xa8, 0xc8, 0xec], // blue
    [0xff, 0xc9, 0x9e], // peach
    [0xb8, 0xe0, 0xa8], // green
    [0xd4, 0xb8, 0xe8], // lavender
    [0xf9, 0xe2, 0x9c], // butter
    [0xf4, 0xa6, 0xa6], // salmon
    [0xa8, 0xdc, 0xd1], // mint
    [0xf8, 0xbb, 0xd9], // pink
    [0xd9, 0xc2, 0xa7], // sand
    [0xb4, 0xbc, 0xf0], // periwinkle
    [0xdc, 0xed, 0x9e], // lime
    [0xc5, 0xd0, 0xd8], // slate
];

#[derive(Serialize, Deserialize, Clone)]
struct Project {
    id: u64,
    name: String,
    color: [u8; 3],
}

impl Project {
    fn color32(&self) -> Color32 {
        Color32::from_rgb(self.color[0], self.color[1], self.color[2])
    }
}

/// Everything that gets persisted between runs.
#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct Data {
    projects: Vec<Project>,
    next_id: u64,
    /// Per date: 96 slots of 15 minutes, each optionally assigned to a project id.
    days: BTreeMap<NaiveDate, Vec<Option<u64>>>,
}

impl Data {
    fn slot(&self, date: NaiveDate, slot: usize) -> Option<u64> {
        self.days.get(&date).and_then(|d| d[slot])
    }

    fn set_slot(&mut self, date: NaiveDate, slot: usize, value: Option<u64>) {
        let day = self
            .days
            .entry(date)
            .or_insert_with(|| vec![None; SLOTS_PER_DAY]);
        day[slot] = value;
        if day.iter().all(Option::is_none) {
            self.days.remove(&date);
        }
    }

    fn project(&self, id: u64) -> Option<&Project> {
        self.projects.iter().find(|p| p.id == id)
    }

    fn project_slots(&self, date: NaiveDate, id: u64) -> usize {
        self.days
            .get(&date)
            .map_or(0, |d| d.iter().filter(|s| **s == Some(id)).count())
    }

    fn filled_slots(&self, date: NaiveDate) -> usize {
        self.days
            .get(&date)
            .map_or(0, |d| d.iter().filter(|s| s.is_some()).count())
    }

    fn add_project(&mut self, name: String) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        let color = PALETTE[self.projects.len() % PALETTE.len()];
        self.projects.push(Project { id, name, color });
        id
    }

    fn delete_project(&mut self, id: u64) {
        self.projects.retain(|p| p.id != id);
        for day in self.days.values_mut() {
            for slot in day.iter_mut().filter(|s| **s == Some(id)) {
                *slot = None;
            }
        }
        self.days.retain(|_, d| d.iter().any(Option::is_some));
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Tool {
    Paint(u64),
    Erase,
}

/// An in-progress paint drag.
struct PaintStroke {
    value: Option<u64>,
    last: (usize, usize),
}

struct TimesheetApp {
    data: Data,
    week_start: NaiveDate,
    tool: Option<Tool>,
    stroke: Option<PaintStroke>,
    new_project_name: String,
    center_on_now: bool,
    settings: Settings,
    show_settings: bool,
    /// JSON last written to (or read from) the data file, to skip redundant writes.
    last_saved: Vec<u8>,
    /// Set when the data file exists but couldn't be read, so we never overwrite it.
    save_blocked: bool,
    error: Option<String>,
    /// One-off message shown in a dialog until dismissed.
    notice: Option<String>,
}

fn monday_of(date: NaiveDate) -> NaiveDate {
    date - Duration::days(date.weekday().num_days_from_monday() as i64)
}

fn hours(slots: usize) -> String {
    format!("{:.2} h", slots as f32 / 4.0)
}

/// Formats minutes since midnight, e.g. `14:30` or `2:30 PM`.
fn clock(minutes: usize, clock_24h: bool) -> String {
    let (h, m) = (minutes / 60, minutes % 60);
    if clock_24h {
        format!("{h:02}:{m:02}")
    } else {
        let suffix = if h < 12 { "AM" } else { "PM" };
        format!("{}:{m:02} {suffix}", (h + 11) % 12 + 1)
    }
}

fn text_color_for(bg: Color32) -> Color32 {
    let lum = 0.299 * bg.r() as f32 + 0.587 * bg.g() as f32 + 0.114 * bg.b() as f32;
    if lum > 150.0 {
        Color32::BLACK
    } else {
        Color32::WHITE
    }
}

impl TimesheetApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let settings = Settings::load();
        cc.egui_ctx.set_theme(settings.theme);
        let mut app = Self {
            data: Data::default(),
            week_start: monday_of(Local::now().date_naive()),
            tool: None,
            stroke: None,
            new_project_name: String::new(),
            center_on_now: true,
            settings,
            show_settings: false,
            last_saved: Vec::new(),
            save_blocked: false,
            error: None,
            notice: None,
        };
        app.load_data(cc);
        app
    }

    fn load_data(&mut self, cc: &eframe::CreationContext<'_>) {
        let dir = self.settings.data_dir();
        let path = storage::data_path(&dir);
        let backup = storage::backup_path(&dir);
        match storage::read_json::<Data>(&path) {
            Ok(Some(data)) => {
                self.last_saved = serde_json::to_vec_pretty(&data).unwrap_or_default();
                self.data = data;
                // Known-good copy to fall back on if the file is ever damaged.
                let _ = storage::write_bytes(&backup, &self.last_saved);
            }
            Ok(None) => {
                // First run with file storage: pick up data saved by the previous version.
                if let Some(data) = cc
                    .storage
                    .and_then(|s| eframe::get_value(s, LEGACY_STORAGE_KEY))
                {
                    self.data = data;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::InvalidData => {
                let Ok(Some(data)) = storage::read_json::<Data>(&backup) else {
                    self.block_saving(&path, &e);
                    return;
                };
                // Keep the damaged file for inspection rather than overwriting it.
                let damaged = dir.join(format!(
                    "timesheet.damaged-{}.json",
                    Local::now().format("%Y%m%d-%H%M%S")
                ));
                if let Err(e) = std::fs::rename(&path, &damaged) {
                    self.block_saving(&path, &e);
                    return;
                }
                self.data = data;
                self.notice = Some(format!(
                    "timesheet.json was damaged, so it was restored from the backup made when \
                     the app was last opened. Changes made after that are missing.\n\n\
                     The damaged file was kept as:\n{}",
                    damaged.display()
                ));
            }
            Err(e) => self.block_saving(&path, &e),
        }
    }

    /// Refuses to save so an unreadable data file is never overwritten.
    fn block_saving(&mut self, path: &std::path::Path, e: &std::io::Error) {
        self.save_blocked = true;
        self.error = Some(format!(
            "Couldn't read {}: {e}. Changes won't be saved until you pick another folder.",
            path.display()
        ));
    }

    fn notice_window(&mut self, ctx: &egui::Context) {
        let Some(notice) = &self.notice else {
            return;
        };
        let mut dismissed = false;
        egui::Window::new("Data restored")
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label(notice);
                ui.add_space(8.0);
                dismissed = ui.button("OK").clicked();
            });
        if dismissed {
            self.notice = None;
        }
    }

    fn save_data(&mut self) {
        if self.save_blocked {
            return;
        }
        let Ok(json) = serde_json::to_vec_pretty(&self.data) else {
            return;
        };
        if json == self.last_saved {
            return;
        }
        let path = storage::data_path(&self.settings.data_dir());
        match storage::write_bytes(&path, &json) {
            Ok(()) => {
                self.last_saved = json;
                self.error = None;
            }
            Err(e) => self.error = Some(format!("Couldn't save {}: {e}", path.display())),
        }
    }

    fn save_settings(&mut self) {
        if let Err(e) = self.settings.save() {
            self.error = Some(format!("Couldn't save settings: {e}"));
        }
    }

    /// Switches the data folder. If the folder already has a timesheet it is loaded;
    /// otherwise the current data is written there. The old file is left untouched.
    fn set_data_dir(&mut self, dir: Option<PathBuf>) {
        let new_dir = dir.clone().unwrap_or_else(storage::default_dir);
        if new_dir == self.settings.data_dir() {
            return;
        }
        self.save_data();
        let path = storage::data_path(&new_dir);
        match storage::read_json::<Data>(&path) {
            Ok(Some(data)) => {
                self.last_saved = serde_json::to_vec_pretty(&data).unwrap_or_default();
                self.data = data;
                self.tool = None;
            }
            Ok(None) => {
                if let Err(e) = storage::write_json(&path, &self.data) {
                    self.error = Some(format!("Couldn't write to {}: {e}", new_dir.display()));
                    return;
                }
                self.last_saved = serde_json::to_vec_pretty(&self.data).unwrap_or_default();
            }
            Err(e) => {
                self.error = Some(format!("Couldn't read {}: {e}", path.display()));
                return;
            }
        }
        self.save_blocked = false;
        self.error = None;
        self.settings.data_dir = dir;
        self.save_settings();
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_settings;
        let mut new_dir = None;
        egui::Window::new("Settings")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                let before = self.settings.clone();

                ui.strong("Theme");
                ui.horizontal(|ui| {
                    ui.radio_value(&mut self.settings.theme, Theme::System, "System");
                    ui.radio_value(&mut self.settings.theme, Theme::Light, "Light");
                    ui.radio_value(&mut self.settings.theme, Theme::Dark, "Dark");
                });

                ui.add_space(8.0);
                ui.strong("Clock");
                let now = Local::now();
                let minutes = (now.hour() * 60 + now.minute()) as usize;
                ui.radio_value(
                    &mut self.settings.clock_24h,
                    true,
                    format!("24-hour ({})", clock(minutes, true)),
                );
                ui.radio_value(
                    &mut self.settings.clock_24h,
                    false,
                    format!("12-hour ({})", clock(minutes, false)),
                );

                if self.settings != before {
                    if self.settings.theme != before.theme {
                        ui.ctx().set_theme(self.settings.theme);
                    }
                    self.save_settings();
                }

                ui.add_space(8.0);
                ui.strong("Data folder");
                let dir = self.settings.data_dir();
                ui.horizontal(|ui| {
                    ui.monospace(dir.display().to_string());
                    if self.settings.data_dir.is_none() {
                        ui.weak("(default)");
                    }
                });
                ui.horizontal(|ui| {
                    if ui.button("Choose…").clicked() {
                        if let Some(picked) =
                            rfd::FileDialog::new().set_directory(&dir).pick_folder()
                        {
                            new_dir = Some(Some(picked));
                        }
                    }
                    if ui
                        .add_enabled(
                            self.settings.data_dir.is_some(),
                            egui::Button::new("Use default"),
                        )
                        .clicked()
                    {
                        new_dir = Some(None);
                    }
                    if ui.button("Open").clicked() {
                        storage::reveal(&dir);
                    }
                });
                ui.weak(
                    "If the chosen folder already has a timesheet.json it's loaded,\n\
                     otherwise your current data is copied there.\n\
                     Settings always stay in the default folder.",
                );

                if let Some(err) = &self.error {
                    ui.add_space(6.0);
                    ui.colored_label(ui.visuals().error_fg_color, err);
                }
            });
        self.show_settings = open;
        if let Some(dir) = new_dir {
            self.set_data_dir(dir);
        }
    }

    fn week_dates(&self) -> Vec<NaiveDate> {
        (0..7)
            .map(|i| self.week_start + Duration::days(i))
            .collect()
    }

    fn side_panel(&mut self, ui: &mut egui::Ui) {
        let dates = self.week_dates();

        ui.heading("Projects");
        ui.add_space(4.0);

        ui.horizontal(|ui| {
            let resp = ui.add(
                egui::TextEdit::singleline(&mut self.new_project_name)
                    .hint_text("New project…")
                    .desired_width(ui.available_width() - 50.0),
            );
            let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if (ui.button("Add").clicked() || enter) && !self.new_project_name.trim().is_empty() {
                let id = self
                    .data
                    .add_project(self.new_project_name.trim().to_owned());
                self.new_project_name.clear();
                self.tool = Some(Tool::Paint(id));
                if enter {
                    resp.request_focus();
                }
            }
        });
        ui.add_space(6.0);

        let eraser = ui.selectable_label(self.tool == Some(Tool::Erase), "⌫  Eraser");
        if eraser.clicked() {
            self.tool = toggle(self.tool, Tool::Erase);
        }
        ui.separator();

        let mut delete = None;
        egui::ScrollArea::vertical()
            .id_salt("projects")
            .max_height(ui.available_height() - 140.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                for (i, project) in self.data.projects.iter_mut().enumerate() {
                    let id = project.id;
                    let slots = dates
                        .iter()
                        .filter_map(|d| self.data.days.get(d))
                        .map(|d| d.iter().filter(|s| **s == Some(id)).count())
                        .sum::<usize>();
                    let selected = self.tool == Some(Tool::Paint(id));
                    let row = ui.horizontal(|ui| {
                        ui.color_edit_button_srgb(&mut project.color);
                        let label = if i < 9 {
                            format!("{}  {}", i + 1, project.name)
                        } else {
                            project.name.clone()
                        };
                        let resp = ui.add(
                            egui::Button::selectable(selected, label)
                                .min_size(Vec2::new(ui.available_width() - 60.0, 0.0)),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(hours(slots));
                        });
                        resp
                    });
                    let resp = row.inner;
                    if resp.clicked() {
                        self.tool = toggle(self.tool, Tool::Paint(id));
                    }
                    resp.context_menu(|ui| {
                        ui.label("Name");
                        ui.text_edit_singleline(&mut project.name);
                        ui.separator();
                        if ui.button("🗑  Delete project").clicked() {
                            delete = Some(id);
                            ui.close();
                        }
                    });
                }
                if self.data.projects.is_empty() {
                    ui.weak("Add a project, select it, then paint on the timesheet.");
                }
            });
        if let Some(id) = delete {
            self.data.delete_project(id);
            if self.tool == Some(Tool::Paint(id)) {
                self.tool = None;
            }
        }

        // Week summary pinned to the bottom of the panel.
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            ui.add_space(6.0);
            let total: usize = dates.iter().map(|d| self.data.filled_slots(*d)).sum();
            let total_h = total as f32 / 4.0;
            let remaining = WEEK_TARGET_HOURS - total_h;
            ui.add(
                egui::ProgressBar::new((total_h / WEEK_TARGET_HOURS).min(1.0))
                    .text(format!("{total_h:.2} / {WEEK_TARGET_HOURS:.0} h")),
            );
            if remaining >= 0.0 {
                ui.label(format!("Remaining: {remaining:.2} h"));
            } else {
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    format!("Overtime: {:.2} h", -remaining),
                );
            }
            ui.strong(format!("Week total: {total_h:.2} h"));
            ui.separator();
            ui.weak("Left-drag: paint · Right-drag: erase\n1–9: pick project · Esc: drop tool");
        });
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.button("◀").clicked() {
                self.week_start -= Duration::days(7);
            }
            if ui.button("Today").clicked() {
                self.week_start = monday_of(Local::now().date_naive());
                self.center_on_now = true;
            }
            if ui.button("▶").clicked() {
                self.week_start += Duration::days(7);
            }
            let end = self.week_start + Duration::days(6);
            ui.strong(format!(
                "{} – {}",
                self.week_start.format("%b %-d"),
                end.format("%b %-d, %Y")
            ));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("⚙ Settings").clicked() {
                    self.show_settings = !self.show_settings;
                }
                if let Some(err) = &self.error {
                    ui.colored_label(ui.visuals().error_fg_color, "⚠ Not saved")
                        .on_hover_text(err);
                }
            });
        });
    }

    fn day_headers(&self, ui: &mut egui::Ui, col_w: f32) {
        let (rect, _) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), HEADER_H), Sense::hover());
        let painter = ui.painter_at(rect);
        let today = Local::now().date_naive();
        let visuals = ui.visuals();
        for (i, date) in self.week_dates().into_iter().enumerate() {
            let x = rect.left() + GUTTER_W + i as f32 * col_w;
            let cell = Rect::from_min_size(Pos2::new(x, rect.top()), Vec2::new(col_w, HEADER_H));
            if date == today {
                painter.rect_filled(cell.shrink(1.0), 4.0, visuals.selection.bg_fill);
            }
            painter.text(
                cell.center_top() + Vec2::new(0.0, 4.0),
                Align2::CENTER_TOP,
                date.format("%a %-d").to_string(),
                FontId::proportional(14.0),
                visuals.strong_text_color(),
            );
            painter.text(
                cell.center_bottom() - Vec2::new(0.0, 4.0),
                Align2::CENTER_BOTTOM,
                hours(self.data.filled_slots(date)),
                FontId::proportional(12.0),
                visuals.weak_text_color(),
            );
        }
    }

    /// Projects with time this week, with their slot counts per day.
    fn breakdown_rows(&self) -> Vec<(&Project, Vec<usize>)> {
        let dates = self.week_dates();
        self.data
            .projects
            .iter()
            .map(|p| {
                let per_day = dates
                    .iter()
                    .map(|d| self.data.project_slots(*d, p.id))
                    .collect::<Vec<_>>();
                (p, per_day)
            })
            .filter(|(_, per_day)| per_day.iter().any(|s| *s > 0))
            .collect()
    }

    /// One row per project with time this week, showing its hours under each day.
    /// Clicking a number copies it for pasting into another time tracker.
    fn day_breakdown(&self, ui: &mut egui::Ui, col_w: f32) {
        let rows = self.breakdown_rows();
        if rows.is_empty() {
            return;
        }

        let (rect, _) = ui.allocate_exact_size(
            Vec2::new(ui.available_width(), rows.len() as f32 * ROW_H),
            Sense::hover(),
        );
        let painter = ui.painter_at(rect);
        let visuals = ui.visuals().clone();
        let font = FontId::proportional(12.0);
        for (r, (project, per_day)) in rows.iter().enumerate() {
            let y = rect.top() + r as f32 * ROW_H;
            if r % 2 == 1 {
                painter.rect_filled(
                    Rect::from_min_size(Pos2::new(rect.left(), y), Vec2::new(rect.width(), ROW_H)),
                    0.0,
                    visuals.faint_bg_color,
                );
            }

            // Project name in the gutter, with a color stripe.
            let label = Rect::from_min_size(Pos2::new(rect.left(), y), Vec2::new(GUTTER_W, ROW_H));
            painter.rect_filled(
                Rect::from_min_size(label.min + Vec2::new(0.0, 2.0), Vec2::new(3.0, ROW_H - 4.0)),
                1.0,
                project.color32(),
            );
            painter.with_clip_rect(label).text(
                label.left_center() + Vec2::new(6.0, 0.0),
                Align2::LEFT_CENTER,
                &project.name,
                font.clone(),
                visuals.text_color(),
            );
            ui.interact(
                label,
                Id::new(("breakdown_name", project.id)),
                Sense::hover(),
            )
            .on_hover_text(&project.name);

            for (i, slots) in per_day.iter().enumerate().filter(|(_, s)| **s > 0) {
                let cell = Rect::from_min_size(
                    Pos2::new(rect.left() + GUTTER_W + i as f32 * col_w, y),
                    Vec2::new(col_w, ROW_H),
                );
                let value = format!("{:.2}", *slots as f32 / 4.0);
                let resp = ui
                    .interact(cell, Id::new(("breakdown", project.id, i)), Sense::click())
                    .on_hover_cursor(CursorIcon::PointingHand)
                    .on_hover_text(format!("{} — click to copy {value}", project.name));
                if resp.hovered() {
                    painter.rect_filled(
                        cell.shrink(1.0),
                        3.0,
                        visuals.widgets.hovered.weak_bg_fill,
                    );
                }
                if resp.clicked() {
                    ui.ctx().copy_text(value.clone());
                }
                let galley = painter.layout_no_wrap(value, font.clone(), visuals.text_color());
                let dot_r = 4.0;
                let total_w = dot_r * 2.0 + 5.0 + galley.size().x;
                let left = cell.center().x - total_w / 2.0;
                painter.circle_filled(
                    Pos2::new(left + dot_r, cell.center().y),
                    dot_r,
                    project.color32(),
                );
                painter.galley(
                    Pos2::new(
                        left + dot_r * 2.0 + 5.0,
                        cell.center().y - galley.size().y / 2.0,
                    ),
                    galley,
                    visuals.text_color(),
                );
            }
        }
        painter.hline(
            rect.x_range(),
            rect.top() + 0.5,
            Stroke::new(1.0, visuals.widgets.noninteractive.bg_stroke.color),
        );
    }

    fn grid(&mut self, ui: &mut egui::Ui, col_w: f32) {
        let height = SLOTS_PER_DAY as f32 * ROW_H;
        let (rect, resp) = ui.allocate_exact_size(
            Vec2::new(ui.available_width(), height),
            Sense::click_and_drag(),
        );
        let dates = self.week_dates();
        let grid_left = rect.left() + GUTTER_W;
        let cell_at = |pos: Pos2| -> (usize, usize) {
            let col = ((pos.x - grid_left) / col_w).floor().clamp(0.0, 6.0) as usize;
            let row = ((pos.y - rect.top()) / ROW_H)
                .floor()
                .clamp(0.0, SLOTS_PER_DAY as f32 - 1.0) as usize;
            (col, row)
        };

        // --- Painting interaction ---
        let pointer = ui.input(|i| i.pointer.interact_pos());
        let (primary, secondary) =
            ui.input(|i| (i.pointer.primary_down(), i.pointer.secondary_down()));
        if resp.is_pointer_button_down_on() && (primary || secondary) {
            if let Some(pos) = pointer {
                let (col, row) = cell_at(pos);
                if self.stroke.is_none() && pos.x >= grid_left {
                    let current = self.data.slot(dates[col], row);
                    let value = if secondary {
                        Some(None)
                    } else {
                        match self.tool {
                            // Starting a stroke on the same project toggles it off.
                            Some(Tool::Paint(id)) if current == Some(id) => Some(None),
                            Some(Tool::Paint(id)) => Some(Some(id)),
                            Some(Tool::Erase) => Some(None),
                            None => {
                                // Eyedropper: clicking a block with no tool picks its project
                                // (and keeps painting it if you drag).
                                current.map(|id| {
                                    self.tool = Some(Tool::Paint(id));
                                    Some(id)
                                })
                            }
                        }
                    };
                    if let Some(value) = value {
                        self.stroke = Some(PaintStroke {
                            value,
                            last: (col, row),
                        });
                        self.data.set_slot(dates[col], row, value);
                    }
                }
                if let Some(stroke) = &mut self.stroke {
                    // Fill every cell between the last and current position so fast drags leave no gaps.
                    let (c0, r0) = (stroke.last.0 as f32, stroke.last.1 as f32);
                    let steps = (col as f32 - c0).abs().max((row as f32 - r0).abs()) as usize;
                    for s in 1..=steps {
                        let t = s as f32 / steps as f32;
                        let c = (c0 + (col as f32 - c0) * t).round() as usize;
                        let r = (r0 + (row as f32 - r0) * t).round() as usize;
                        self.data.set_slot(dates[c], r, stroke.value);
                    }
                    stroke.last = (col, row);
                }

                // Auto-scroll while dragging past the visible edge.
                let clip = ui.clip_rect();
                if pos.y > clip.bottom() - 10.0 {
                    ui.scroll_with_delta(Vec2::new(0.0, -(pos.y - clip.bottom() + 10.0)));
                } else if pos.y < clip.top() + 10.0 {
                    ui.scroll_with_delta(Vec2::new(0.0, clip.top() + 10.0 - pos.y));
                }
            }
        } else {
            self.stroke = None;
        }

        // --- Drawing ---
        let painter = ui.painter_at(rect);
        let visuals = ui.visuals();
        let line_faint = visuals
            .widgets
            .noninteractive
            .bg_stroke
            .color
            .gamma_multiply(0.35);
        let line_hour = visuals.widgets.noninteractive.bg_stroke.color;
        let text_weak = visuals.weak_text_color();

        // Off-hours shading and time labels.
        for hour in 0..24 {
            let y = rect.top() + hour as f32 * 4.0 * ROW_H;
            if !(9..17).contains(&hour) {
                painter.rect_filled(
                    Rect::from_min_max(
                        Pos2::new(grid_left, y),
                        Pos2::new(rect.right(), y + 4.0 * ROW_H),
                    ),
                    0.0,
                    visuals.faint_bg_color,
                );
            }
            painter.text(
                Pos2::new(grid_left - 6.0, y + 2.0),
                Align2::RIGHT_TOP,
                if self.settings.clock_24h {
                    format!("{hour:02}:00")
                } else {
                    clock(hour * 60, false).replace(":00", "")
                },
                FontId::monospace(11.0),
                text_weak,
            );
        }

        // Project blocks, merged into contiguous runs so labels show once.
        for (col, date) in dates.iter().enumerate() {
            let Some(day) = self.data.days.get(date) else {
                continue;
            };
            let x = grid_left + col as f32 * col_w;
            let mut row = 0;
            while row < SLOTS_PER_DAY {
                let Some(id) = day[row] else {
                    row += 1;
                    continue;
                };
                let start = row;
                while row < SLOTS_PER_DAY && day[row] == Some(id) {
                    row += 1;
                }
                let Some(project) = self.data.project(id) else {
                    continue;
                };
                let block = Rect::from_min_size(
                    Pos2::new(x, rect.top() + start as f32 * ROW_H),
                    Vec2::new(col_w, (row - start) as f32 * ROW_H),
                )
                .shrink2(Vec2::new(2.0, 1.0));
                let color = project.color32();
                painter.rect_filled(block, CornerRadius::same(3), color);
                painter.with_clip_rect(block.intersect(rect)).text(
                    block.left_top() + Vec2::new(4.0, 2.0),
                    Align2::LEFT_TOP,
                    format!("{}  {}", project.name, hours(row - start)),
                    FontId::proportional(11.0),
                    text_color_for(color),
                );
            }
        }

        // Grid lines.
        for slot in 0..=SLOTS_PER_DAY {
            let y = rect.top() + slot as f32 * ROW_H;
            let color = if slot % 4 == 0 { line_hour } else { line_faint };
            painter.hline(grid_left..=rect.right(), y, Stroke::new(1.0, color));
        }
        for col in 0..=7 {
            let x = grid_left + col as f32 * col_w;
            painter.vline(x, rect.y_range(), Stroke::new(1.0, line_hour));
        }

        // Hover highlight + tool indicator on the cursor.
        if let Some(pos) = resp.hover_pos().filter(|p| p.x >= grid_left) {
            let (col, row) = cell_at(pos);
            let cell = Rect::from_min_size(
                Pos2::new(
                    grid_left + col as f32 * col_w,
                    rect.top() + row as f32 * ROW_H,
                ),
                Vec2::new(col_w, ROW_H),
            );
            painter.rect_stroke(
                cell,
                2.0,
                Stroke::new(1.5, visuals.strong_text_color()),
                StrokeKind::Inside,
            );
            let time = clock(row * 15, self.settings.clock_24h);
            let fg = ui
                .ctx()
                .layer_painter(LayerId::new(Order::Tooltip, Id::new("cursor")));
            let badge = match self.tool {
                Some(Tool::Paint(id)) => self
                    .data
                    .project(id)
                    .map(|p| (Some(p.color32()), format!("{}  {time}", p.name))),
                Some(Tool::Erase) => Some((None, format!("⌫  {time}"))),
                None => None,
            };
            if let Some((swatch, label)) = badge {
                ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
                let galley = fg.layout_no_wrap(
                    label,
                    FontId::proportional(12.0),
                    visuals.strong_text_color(),
                );
                let pad = Vec2::new(6.0, 4.0);
                let swatch_w = if swatch.is_some() { 16.0 } else { 0.0 };
                let size = galley.size() + Vec2::new(swatch_w, 0.0) + pad * 2.0;
                // Sit below-right of the cursor, flipping to stay on screen.
                let screen = ui.ctx().content_rect();
                let mut min = pos + Vec2::new(14.0, 14.0);
                if min.x + size.x > screen.right() {
                    min.x = pos.x - 14.0 - size.x;
                }
                if min.y + size.y > screen.bottom() {
                    min.y = pos.y - 14.0 - size.y;
                }
                let bg = Rect::from_min_size(min, size);
                fg.add(
                    visuals
                        .popup_shadow
                        .as_shape(bg, visuals.menu_corner_radius),
                );
                fg.rect(
                    bg,
                    visuals.menu_corner_radius,
                    visuals.window_fill,
                    visuals.window_stroke,
                    StrokeKind::Outside,
                );
                if let Some(color) = swatch {
                    fg.circle(
                        Pos2::new(bg.left() + pad.x + 6.0, bg.center().y),
                        6.0,
                        color,
                        Stroke::new(1.0, visuals.strong_text_color()),
                    );
                }
                fg.galley(
                    bg.min + pad + Vec2::new(swatch_w, 0.0),
                    galley,
                    visuals.strong_text_color(),
                );
            }
        }

        // Current time line.
        let now = Local::now();
        if let Some(col) = dates.iter().position(|d| *d == now.date_naive()) {
            let mins = now.hour() as f32 * 60.0 + now.minute() as f32;
            let y = rect.top() + mins / 15.0 * ROW_H;
            let x0 = grid_left + col as f32 * col_w;
            let red = Color32::from_rgb(0xe0, 0x3c, 0x31);
            painter.hline(x0..=x0 + col_w, y, Stroke::new(2.0, red));
            painter.circle_filled(Pos2::new(x0, y), 4.0, red);
        }
    }
}

fn toggle(current: Option<Tool>, tool: Tool) -> Option<Tool> {
    if current == Some(tool) {
        None
    } else {
        Some(tool)
    }
}

impl eframe::App for TimesheetApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Keyboard shortcuts.
        ui.input(|i| {
            if i.key_pressed(egui::Key::Escape) {
                self.tool = None;
            }
        });
        if !ui.ctx().egui_wants_keyboard_input() {
            let keys = [
                egui::Key::Num1,
                egui::Key::Num2,
                egui::Key::Num3,
                egui::Key::Num4,
                egui::Key::Num5,
                egui::Key::Num6,
                egui::Key::Num7,
                egui::Key::Num8,
                egui::Key::Num9,
            ];
            for (n, key) in keys.iter().enumerate() {
                if ui.input(|i| i.key_pressed(*key)) {
                    if let Some(p) = self.data.projects.get(n) {
                        self.tool = Some(Tool::Paint(p.id));
                    }
                }
            }
        }

        let side_w = (ui.available_width() * 0.2).max(240.0);
        egui::Panel::right("projects_panel")
            .resizable(false)
            .exact_size(side_w)
            .show(ui, |ui| self.side_panel(ui));

        egui::CentralPanel::default().show(ui, |ui| {
            self.toolbar(ui);
            ui.add_space(4.0);
            let col_w = (ui.available_width() - GUTTER_W) / 7.0;
            self.day_headers(ui, col_w);

            // The breakdown sits below the grid; the grid gives up height from its bottom
            // edge as rows are added, so its scroll position doesn't move.
            let breakdown_h = self.breakdown_rows().len() as f32 * ROW_H;
            let grid_h = (ui.available_height() - breakdown_h).max(ROW_H * 4.0);
            let mut area = egui::ScrollArea::vertical()
                .id_salt("grid")
                .max_height(grid_h)
                .auto_shrink(false)
                .scroll_source(egui::scroll_area::ScrollSource {
                    scroll_bar: true,
                    mouse_wheel: true,
                    ..egui::scroll_area::ScrollSource::NONE
                });
            if self.center_on_now {
                self.center_on_now = false;
                let now = Local::now();
                let mins = now.hour() as f32 * 60.0 + now.minute() as f32;
                let total = SLOTS_PER_DAY as f32 * ROW_H;
                let offset =
                    (mins / 15.0 * ROW_H - grid_h / 2.0).clamp(0.0, (total - grid_h).max(0.0));
                area = area.vertical_scroll_offset(offset);
            }
            area.show(ui, |ui| self.grid(ui, col_w));
            self.day_breakdown(ui, col_w);
        });

        self.settings_window(ui.ctx());
        self.notice_window(ui.ctx());

        // Keep the "now" line moving.
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_secs(30));
    }

    fn save(&mut self, _storage: &mut dyn eframe::Storage) {
        self.save_data();
    }

    fn auto_save_interval(&self) -> std::time::Duration {
        std::time::Duration::from_secs(3)
    }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Timesheet")
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([800.0, 500.0]),
        ..Default::default()
    };
    eframe::run_native(
        storage::APP_ID,
        options,
        Box::new(|cc| Ok(Box::new(TimesheetApp::new(cc)))),
    )
}

#[cfg(test)]
mod tests {
    use super::clock;

    #[test]
    fn clock_formats() {
        assert_eq!(clock(0, true), "00:00");
        assert_eq!(clock(14 * 60 + 30, true), "14:30");
        assert_eq!(clock(0, false), "12:00 AM");
        assert_eq!(clock(9 * 60 + 15, false), "9:15 AM");
        assert_eq!(clock(12 * 60, false), "12:00 PM");
        assert_eq!(clock(23 * 60 + 45, false), "11:45 PM");
    }
}
