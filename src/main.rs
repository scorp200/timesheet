// Hide the console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod storage;
mod theme;
mod widgets;

use std::{collections::BTreeMap, ops::Range, path::PathBuf};

use chrono::{Datelike, Duration, Local, NaiveDate, Timelike};
use eframe::egui::{
    self, Align2, Color32, CornerRadius, CursorIcon, FontId, Id, LayerId, Order, Pos2, Rect, Sense,
    Stroke, StrokeKind, Vec2,
};
use serde::{Deserialize, Serialize};
use storage::{Settings, Theme};
use theme::{border_color_for, text_color_for};
use widgets::{FlatButton, ProgressTrack, button, dialog_header, text_field};

const SLOTS_PER_DAY: usize = 24 * 4;
const ROW_H: f32 = 18.0;
const HEADER_H: f32 = 40.0;
const GUTTER_W: f32 = 80.0;
const WEEK_TARGET_HOURS: f32 = 40.0;
/// Shown for the eraser tool. Icons must have a drawn glyph in egui's bundled fonts:
/// some characters (like ✕) are listed by a font but drawn as nothing.
const ERASER_ICON: &str = "🗙";
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
        if is_empty_day(day) {
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
        self.days.retain(|_, day| {
            for slot in day.iter_mut().filter(|s| **s == Some(id)) {
                *slot = None;
            }
            !is_empty_day(day)
        });
    }
}

fn is_empty_day(day: &[Option<u64>]) -> bool {
    day.iter().all(Option::is_none)
}

#[derive(Clone, Copy, PartialEq)]
enum Tool {
    Paint(u64),
    Erase,
}

/// An in-progress paint drag.
struct PaintStroke {
    /// What every slot the drag passes over is set to; `None` erases.
    value: Option<u64>,
    last: (usize, usize),
}

/// Screen geometry of the week grid.
#[derive(Clone, Copy)]
struct GridGeom {
    /// The whole grid, including the time gutter on the left.
    rect: Rect,
    col_w: f32,
}

impl GridGeom {
    /// Left edge of the first day column.
    fn left(&self) -> f32 {
        self.rect.left() + GUTTER_W
    }

    fn col_x(&self, col: usize) -> f32 {
        self.left() + col as f32 * self.col_w
    }

    /// The area covered by `rows` in day column `col`.
    fn cells(&self, col: usize, rows: Range<usize>) -> Rect {
        Rect::from_min_size(
            Pos2::new(self.col_x(col), self.rect.top() + rows.start as f32 * ROW_H),
            Vec2::new(self.col_w, rows.len() as f32 * ROW_H),
        )
    }

    /// The (day, slot) under `pos`, clamped to the grid.
    fn cell_at(&self, pos: Pos2) -> (usize, usize) {
        let col = ((pos.x - self.left()) / self.col_w).floor().clamp(0.0, 6.0) as usize;
        let row = ((pos.y - self.rect.top()) / ROW_H)
            .floor()
            .clamp(0.0, SLOTS_PER_DAY as f32 - 1.0) as usize;
        (col, row)
    }
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

fn slots_to_hours(slots: usize) -> f32 {
    slots as f32 / 4.0
}

fn hours(slots: usize) -> String {
    format!("{:.2} h", slots_to_hours(slots))
}

/// Minutes since local midnight, right now.
fn now_minutes() -> f32 {
    let now = Local::now();
    now.hour() as f32 * 60.0 + now.minute() as f32
}

/// Vertical offset of a time of day within the grid.
fn minutes_to_y(minutes: f32) -> f32 {
    minutes / 15.0 * ROW_H
}

/// Converts a 0..24 hour to 12-hour form, e.g. `13` to `(1, "PM")`.
fn twelve_hour(h: usize) -> (usize, &'static str) {
    ((h + 11) % 12 + 1, if h < 12 { "AM" } else { "PM" })
}

/// Formats minutes since midnight, e.g. `14:30` or `2:30 PM`.
fn clock(minutes: usize, clock_24h: bool) -> String {
    let (h, m) = (minutes / 60, minutes % 60);
    if clock_24h {
        format!("{h:02}:{m:02}")
    } else {
        let (h, suffix) = twelve_hour(h);
        format!("{h}:{m:02} {suffix}")
    }
}

/// Label for the start of an hour in the grid gutter, e.g. `14:00` or `2 PM`.
fn hour_label(hour: usize, clock_24h: bool) -> String {
    if clock_24h {
        clock(hour * 60, true)
    } else {
        let (h, suffix) = twelve_hour(hour);
        format!("{h} {suffix}")
    }
}

impl TimesheetApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let settings = Settings::load();
        cc.egui_ctx.set_theme(settings.theme);
        theme::install(&cc.egui_ctx);
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
                self.adopt_data(data);
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
                let stamp = Local::now().format("%Y%m%d-%H%M%S").to_string();
                let damaged = storage::damaged_path(&dir, &stamp);
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

    /// Replaces the in-memory data with what was just read from disk.
    fn adopt_data(&mut self, data: Data) {
        self.last_saved = serde_json::to_vec_pretty(&data).unwrap_or_default();
        self.data = data;
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
                dismissed = button(ui, "OK").clicked();
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
                self.adopt_data(data);
                self.tool = None;
            }
            Ok(None) => {
                let json = serde_json::to_vec_pretty(&self.data).unwrap_or_default();
                if let Err(e) = storage::write_bytes(&path, &json) {
                    self.error = Some(format!("Couldn't write to {}: {e}", new_dir.display()));
                    return;
                }
                self.last_saved = json;
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
        if !self.show_settings {
            return;
        }
        let mut close = false;
        let mut new_dir = None;
        egui::Window::new("Settings")
            .title_bar(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                close =
                    dialog_header(ui, "Settings") || ui.input(|i| i.key_pressed(egui::Key::Escape));
                let before = self.settings.clone();

                ui.strong("Theme");
                ui.horizontal(|ui| {
                    ui.radio_value(&mut self.settings.theme, Theme::System, "System");
                    ui.radio_value(&mut self.settings.theme, Theme::Light, "Light");
                    ui.radio_value(&mut self.settings.theme, Theme::Dark, "Dark");
                });

                ui.add_space(8.0);
                ui.strong("Clock");
                let minutes = now_minutes() as usize;
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
                    if button(ui, "Choose…").clicked()
                        && let Some(picked) =
                            rfd::FileDialog::new().set_directory(&dir).pick_folder()
                    {
                        new_dir = Some(Some(picked));
                    }
                    if ui
                        .add_enabled(
                            self.settings.data_dir.is_some(),
                            FlatButton::new("Use default"),
                        )
                        .clicked()
                    {
                        new_dir = Some(None);
                    }
                    if button(ui, "Open").clicked() {
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
        if close {
            self.show_settings = false;
        }
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

        // Laid out first so the project list gets whatever height is left.
        egui::Panel::bottom("week_summary")
            .frame(egui::Frame::NONE)
            .show_separator_line(false)
            .show(ui, |ui| self.week_summary(ui, &dates));

        ui.heading("Projects");
        ui.add_space(4.0);

        // Right to left so the text field takes whatever width the button leaves.
        let row = Vec2::new(ui.available_width(), widgets::HEIGHT);
        ui.allocate_ui_with_layout(
            row,
            egui::Layout::right_to_left(egui::Align::Center),
            |ui| {
                let add = button(ui, "Add").clicked();
                let resp = text_field(ui, &mut self.new_project_name, "New project…");
                let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if (add || enter) && !self.new_project_name.trim().is_empty() {
                    let id = self
                        .data
                        .add_project(self.new_project_name.trim().to_owned());
                    self.new_project_name.clear();
                    self.tool = Some(Tool::Paint(id));
                    if enter {
                        resp.request_focus();
                    }
                }
            },
        );
        ui.add_space(6.0);

        let eraser = ui.selectable_label(
            self.tool == Some(Tool::Erase),
            format!("{ERASER_ICON}  Eraser"),
        );
        if eraser.clicked() {
            self.tool = toggle(self.tool, Tool::Erase);
        }
        ui.separator();

        let week_slots: Vec<usize> = self
            .data
            .projects
            .iter()
            .map(|p| {
                dates
                    .iter()
                    .map(|d| self.data.project_slots(*d, p.id))
                    .sum()
            })
            .collect();
        let mut delete = None;
        egui::ScrollArea::vertical()
            .id_salt("projects")
            .auto_shrink([false, true])
            .show(ui, |ui| {
                for (i, (project, slots)) in
                    self.data.projects.iter_mut().zip(week_slots).enumerate()
                {
                    let id = project.id;
                    let selected = self.tool == Some(Tool::Paint(id));
                    let row = ui.horizontal(|ui| {
                        ui.color_edit_button_srgb(&mut project.color);
                        let label = if i < 9 {
                            format!("{}  {}", i + 1, project.name)
                        } else {
                            project.name.clone()
                        };
                        ui.add(
                            egui::Button::selectable(selected, label)
                                .right_text(hours(slots))
                                .min_size(Vec2::new(ui.available_width(), 0.0)),
                        )
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
    }

    /// Hint text and the week's progress, pinned to the bottom of the side panel.
    fn week_summary(&self, ui: &mut egui::Ui, dates: &[NaiveDate]) {
        ui.weak("Left-drag: paint · Right-drag: erase\nKeys 1 to 9: pick project · Esc: drop tool");
        ui.separator();
        ui.add_space(4.0);

        let total_h = slots_to_hours(dates.iter().map(|d| self.data.filled_slots(*d)).sum());
        let remaining = WEEK_TARGET_HOURS - total_h;
        let mut track = ProgressTrack::new(total_h / WEEK_TARGET_HOURS).labels(
            "This week",
            format!("{total_h:.2} / {WEEK_TARGET_HOURS:.0} h"),
        );
        if remaining < 0.0 {
            track = track.color(ui.visuals().warn_fg_color);
        }
        ui.add(track);
        ui.add_space(4.0);
        if remaining >= 0.0 {
            ui.weak(format!("{remaining:.2} h remaining"));
        } else {
            ui.colored_label(
                ui.visuals().warn_fg_color,
                format!("{:.2} h overtime", -remaining),
            );
        }
        ui.add_space(6.0);
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.add(FlatButton::icon("◀")).clicked() {
                self.week_start -= Duration::days(7);
            }
            if button(ui, "Today").clicked() {
                self.week_start = monday_of(Local::now().date_naive());
                self.center_on_now = true;
            }
            if ui.add(FlatButton::icon("▶")).clicked() {
                self.week_start += Duration::days(7);
            }
            let end = self.week_start + Duration::days(6);
            ui.strong(format!(
                "{} to {}",
                self.week_start.format("%b %-d"),
                end.format("%b %-d, %Y")
            ));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if button(ui, "⚙ Settings").clicked() {
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
                let value = format!("{:.2}", slots_to_hours(*slots));
                let resp = ui
                    .interact(cell, Id::new(("breakdown", project.id, i)), Sense::click())
                    .on_hover_cursor(CursorIcon::PointingHand)
                    .on_hover_text(format!("{}: click to copy {value}", project.name));
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
        let (rect, resp) = ui.allocate_exact_size(
            Vec2::new(ui.available_width(), SLOTS_PER_DAY as f32 * ROW_H),
            Sense::click_and_drag(),
        );
        let geom = GridGeom { rect, col_w };
        let dates = self.week_dates();
        self.handle_painting(ui, &resp, geom, &dates);

        let painter = ui.painter_at(rect);
        self.draw_background(ui, &painter, geom);
        self.draw_blocks(&painter, geom, &dates);

        // Hover highlight + tool indicator on the cursor.
        if let Some(pos) = resp.hover_pos().filter(|p| p.x >= geom.left()) {
            let (col, row) = geom.cell_at(pos);
            painter.rect_stroke(
                geom.cells(col, row..row + 1),
                2.0,
                Stroke::new(1.5, ui.visuals().strong_text_color()),
                StrokeKind::Inside,
            );
            self.cursor_badge(ui, pos, row);
        }

        // Current time line.
        if let Some(col) = dates.iter().position(|d| *d == Local::now().date_naive()) {
            let y = rect.top() + minutes_to_y(now_minutes());
            let x0 = geom.col_x(col);
            let red = Color32::from_rgb(0xe0, 0x3c, 0x31);
            painter.hline(x0..=x0 + col_w, y, Stroke::new(2.0, red));
            painter.circle_filled(Pos2::new(x0, y), 4.0, red);
        }
    }

    /// Left-drag paints with the current tool, right-drag erases.
    fn handle_painting(
        &mut self,
        ui: &egui::Ui,
        resp: &egui::Response,
        geom: GridGeom,
        dates: &[NaiveDate],
    ) {
        let (primary, secondary) =
            ui.input(|i| (i.pointer.primary_down(), i.pointer.secondary_down()));
        if !(resp.is_pointer_button_down_on() && (primary || secondary)) {
            self.stroke = None;
            return;
        }
        let Some(pos) = ui.input(|i| i.pointer.interact_pos()) else {
            return;
        };
        let (col, row) = geom.cell_at(pos);
        if self.stroke.is_none() && pos.x >= geom.left() {
            let current = self.data.slot(dates[col], row);
            self.stroke = self.start_stroke(current, secondary, (col, row));
            if let Some(stroke) = &self.stroke {
                self.data.set_slot(dates[col], row, stroke.value);
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

    /// Decides what a drag starting on a slot holding `current` will paint.
    /// Returns `None` when the click should do nothing.
    fn start_stroke(
        &mut self,
        current: Option<u64>,
        erase: bool,
        at: (usize, usize),
    ) -> Option<PaintStroke> {
        let value = match (erase, self.tool) {
            (true, _) | (false, Some(Tool::Erase)) => None,
            // Starting a stroke on the same project toggles it off.
            (false, Some(Tool::Paint(id))) if current == Some(id) => None,
            (false, Some(Tool::Paint(id))) => Some(id),
            // Eyedropper: clicking a block with no tool picks its project (and keeps
            // painting it if you drag). Clicking an empty slot does nothing.
            (false, None) => {
                let id = current?;
                self.tool = Some(Tool::Paint(id));
                Some(id)
            }
        };
        Some(PaintStroke { value, last: at })
    }

    /// Off-hours shading, hour labels, and grid lines.
    fn draw_background(&self, ui: &egui::Ui, painter: &egui::Painter, geom: GridGeom) {
        let visuals = ui.visuals();
        let line_hour = visuals.widgets.noninteractive.bg_stroke.color;
        let line_faint = line_hour.gamma_multiply(0.35);
        let rect = geom.rect;

        for hour in 0..24 {
            let y = rect.top() + hour as f32 * 4.0 * ROW_H;
            if !(9..17).contains(&hour) {
                painter.rect_filled(
                    Rect::from_min_max(
                        Pos2::new(geom.left(), y),
                        Pos2::new(rect.right(), y + 4.0 * ROW_H),
                    ),
                    0.0,
                    visuals.faint_bg_color,
                );
            }
            painter.text(
                Pos2::new(geom.left() - 6.0, y + 2.0),
                Align2::RIGHT_TOP,
                hour_label(hour, self.settings.clock_24h),
                FontId::monospace(11.0),
                visuals.weak_text_color(),
            );
        }

        for slot in 0..=SLOTS_PER_DAY {
            let y = rect.top() + slot as f32 * ROW_H;
            let color = if slot % 4 == 0 { line_hour } else { line_faint };
            painter.hline(geom.left()..=rect.right(), y, Stroke::new(1.0, color));
        }
        for col in 0..=7 {
            painter.vline(geom.col_x(col), rect.y_range(), Stroke::new(1.0, line_hour));
        }
    }

    /// Project blocks, merged into contiguous runs so labels show once.
    fn draw_blocks(&self, painter: &egui::Painter, geom: GridGeom, dates: &[NaiveDate]) {
        for (col, date) in dates.iter().enumerate() {
            let Some(day) = self.data.days.get(date) else {
                continue;
            };
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
                let block = geom.cells(col, start..row).shrink2(Vec2::new(2.0, 1.0));
                let color = project.color32();
                painter.rect(
                    block,
                    CornerRadius::same(3),
                    color,
                    Stroke::new(1.0, border_color_for(color)),
                    StrokeKind::Inside,
                );
                // Name on top with the duration under it; a single slot fits only one line.
                let clip = painter.with_clip_rect(block.shrink(1.0).intersect(geom.rect));
                let fg = text_color_for(color);
                let duration = hours(row - start);
                if row - start == 1 {
                    clip.text(
                        Pos2::new(block.left() + 6.0, block.center().y),
                        Align2::LEFT_CENTER,
                        format!("{}  {duration}", project.name),
                        FontId::proportional(12.0),
                        fg,
                    );
                } else {
                    let name = clip.text(
                        block.left_top() + Vec2::new(6.0, 3.0),
                        Align2::LEFT_TOP,
                        &project.name,
                        FontId::proportional(13.5),
                        fg,
                    );
                    clip.text(
                        Pos2::new(block.left() + 6.0, name.bottom()),
                        Align2::LEFT_TOP,
                        duration,
                        FontId::proportional(12.0),
                        fg.gamma_multiply(0.75),
                    );
                }
            }
        }
    }

    /// A small label next to the cursor showing the active tool and the hovered time.
    fn cursor_badge(&self, ui: &egui::Ui, pos: Pos2, row: usize) {
        let time = clock(row * 15, self.settings.clock_24h);
        let (swatch, label) = match self.tool {
            Some(Tool::Paint(id)) => {
                let Some(p) = self.data.project(id) else {
                    return;
                };
                (Some(p.color32()), format!("{}  {time}", p.name))
            }
            Some(Tool::Erase) => (None, format!("{ERASER_ICON}  {time}")),
            None => return,
        };

        ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
        let visuals = ui.visuals();
        let fg = ui
            .ctx()
            .layer_painter(LayerId::new(Order::Tooltip, Id::new("cursor")));
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
                if ui.input(|i| i.key_pressed(*key))
                    && let Some(p) = self.data.projects.get(n)
                {
                    self.tool = Some(Tool::Paint(p.id));
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
                let total = SLOTS_PER_DAY as f32 * ROW_H;
                let offset = (minutes_to_y(now_minutes()) - grid_h / 2.0)
                    .clamp(0.0, (total - grid_h).max(0.0));
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
    use super::{clock, hour_label};

    #[test]
    fn clock_formats() {
        assert_eq!(clock(0, true), "00:00");
        assert_eq!(clock(14 * 60 + 30, true), "14:30");
        assert_eq!(clock(0, false), "12:00 AM");
        assert_eq!(clock(9 * 60 + 15, false), "9:15 AM");
        assert_eq!(clock(12 * 60, false), "12:00 PM");
        assert_eq!(clock(23 * 60 + 45, false), "11:45 PM");
    }

    #[test]
    fn hour_labels() {
        assert_eq!(hour_label(0, true), "00:00");
        assert_eq!(hour_label(14, true), "14:00");
        assert_eq!(hour_label(0, false), "12 AM");
        assert_eq!(hour_label(12, false), "12 PM");
        assert_eq!(hour_label(23, false), "11 PM");
    }
}
