use eframe::egui::{
    Align, Color32, CornerRadius, CursorIcon, Margin, Pos2, Rect, Response, Sense, Sides,
    StrokeKind, TextEdit, TextStyle, TextWrapMode, Ui, Vec2, Widget, WidgetInfo, WidgetText,
    WidgetType,
};

use crate::theme::{accent, text_color_for};

/// Close button glyph for dialogs.
pub const CLOSE_ICON: &str = "🗙";

/// Height shared by buttons, text fields, and progress bars.
pub const HEIGHT: f32 = 28.0;
const PADDING_X: f32 = 12.0;
const RADIUS: u8 = 6;

/// A rounded, flat button with a fixed height, used for toolbar and form actions.
/// Colors come from the active theme's widget visuals.
pub struct FlatButton {
    text: WidgetText,
    square: bool,
}

impl FlatButton {
    pub fn new(text: impl Into<WidgetText>) -> Self {
        Self {
            text: text.into(),
            square: false,
        }
    }

    /// A square button for a single icon or symbol.
    pub fn icon(text: impl Into<WidgetText>) -> Self {
        Self {
            text: text.into(),
            square: true,
        }
    }
}

impl Widget for FlatButton {
    fn ui(self, ui: &mut Ui) -> Response {
        let galley = self.text.into_galley(
            ui,
            Some(TextWrapMode::Extend),
            f32::INFINITY,
            TextStyle::Button,
        );
        let width = if self.square {
            HEIGHT
        } else {
            (galley.size().x + 2.0 * PADDING_X).max(HEIGHT)
        };
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, HEIGHT), Sense::click());
        resp.widget_info(|| {
            WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), galley.text())
        });

        if ui.is_rect_visible(rect) {
            let widgets = &ui.visuals().widgets;
            let hover =
                ui.ctx()
                    .animate_bool_with_time(resp.id.with("hover"), resp.hovered(), 0.08);
            let fill = if resp.is_pointer_button_down_on() {
                widgets.active.weak_bg_fill
            } else {
                widgets
                    .inactive
                    .weak_bg_fill
                    .lerp_to_gamma(widgets.hovered.weak_bg_fill, hover)
            };
            let stroke = if resp.has_focus() {
                ui.visuals().selection.stroke
            } else {
                widgets.inactive.bg_stroke
            };
            let text_color = if resp.hovered() {
                widgets.hovered.fg_stroke.color
            } else {
                widgets.inactive.fg_stroke.color
            };
            let painter = ui.painter();
            painter.rect(
                rect,
                CornerRadius::same(RADIUS),
                fill,
                stroke,
                StrokeKind::Inside,
            );
            painter.galley(rect.center() - galley.size() / 2.0, galley, text_color);
        }

        if resp.hovered() && ui.is_enabled() {
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        }
        resp
    }
}

/// Shorthand for `ui.add(FlatButton::new(text))`.
pub fn button(ui: &mut Ui, text: impl Into<WidgetText>) -> Response {
    ui.add(FlatButton::new(text))
}

/// Title row for a dialog drawn without egui's title bar, with a close button the
/// same size as other buttons. Returns `true` when the close button is clicked.
pub fn dialog_header(ui: &mut Ui, title: &str) -> bool {
    let (_, closed) = Sides::new().height(HEIGHT).show(
        ui,
        |ui| ui.heading(title),
        |ui| {
            ui.add(FlatButton::icon(CLOSE_ICON))
                .on_hover_text("Close")
                .clicked()
        },
    );
    ui.add_space(4.0);
    closed
}

/// A single-line text field with the same height and rounding as [`FlatButton`].
/// Fills the available width.
pub fn text_field(ui: &mut Ui, text: &mut String, hint: &str) -> Response {
    ui.scope(|ui| {
        // Round the frame to match buttons and stop it growing on hover.
        let widgets = &mut ui.visuals_mut().widgets;
        for state in [
            &mut widgets.inactive,
            &mut widgets.hovered,
            &mut widgets.active,
        ] {
            state.corner_radius = CornerRadius::same(RADIUS);
            state.expansion = 0.0;
        }
        ui.add(
            TextEdit::singleline(text)
                .hint_text(hint)
                .desired_width(ui.available_width())
                .min_size(Vec2::new(0.0, HEIGHT))
                .margin(Margin::symmetric(8, 0))
                .vertical_align(Align::Center),
        )
    })
    .inner
}

/// A progress bar with the same height and rounding as [`FlatButton`], with optional
/// labels pinned to its left and right ends. `fraction` is clamped to 0..=1 and the
/// fill animates when it changes.
pub struct ProgressTrack {
    fraction: f32,
    color: Option<Color32>,
    left: String,
    right: String,
}

impl ProgressTrack {
    pub fn new(fraction: f32) -> Self {
        Self {
            fraction: fraction.clamp(0.0, 1.0),
            color: None,
            left: String::new(),
            right: String::new(),
        }
    }

    /// Fill color; defaults to the theme's accent.
    pub fn color(mut self, color: Color32) -> Self {
        self.color = Some(color);
        self
    }

    /// Labels shown inside the bar at its left and right ends.
    pub fn labels(mut self, left: impl Into<String>, right: impl Into<String>) -> Self {
        self.left = left.into();
        self.right = right.into();
        self
    }
}

impl Widget for ProgressTrack {
    fn ui(self, ui: &mut Ui) -> Response {
        let (rect, resp) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), HEIGHT), Sense::hover());
        if !ui.is_rect_visible(rect) {
            return resp;
        }
        let fraction = ui
            .ctx()
            .animate_value_with_time(resp.id, self.fraction, 0.25);
        let visuals = ui.visuals();
        let radius = CornerRadius::same(RADIUS);
        let fill_color = self.color.unwrap_or_else(|| accent(ui));
        let painter = ui.painter();
        painter.rect_filled(rect, radius, visuals.widgets.inactive.weak_bg_fill);

        // Keep the fill at least as wide as its rounded ends.
        let fill_w = if fraction > 0.0 {
            (rect.width() * fraction).max(2.0 * RADIUS as f32)
        } else {
            0.0
        };
        let fill = Rect::from_min_size(rect.min, Vec2::new(fill_w, HEIGHT));
        if fill_w > 0.0 {
            painter.rect_filled(fill, radius, fill_color);
        }

        // Labels change color where the fill passes under them.
        let font = TextStyle::Button.resolve(ui.style());
        let left = painter.layout_no_wrap(self.left, font.clone(), Color32::PLACEHOLDER);
        let right = painter.layout_no_wrap(self.right, font, Color32::PLACEHOLDER);
        let left_pos = Pos2::new(
            rect.left() + PADDING_X,
            rect.center().y - left.size().y / 2.0,
        );
        let right_pos = Pos2::new(
            rect.right() - PADDING_X - right.size().x,
            rect.center().y - right.size().y / 2.0,
        );
        let empty = Rect::from_min_max(Pos2::new(fill.right(), rect.top()), rect.max);
        let on_fill = text_color_for(fill_color);
        for (clip, strong, normal) in [
            (empty, visuals.strong_text_color(), visuals.text_color()),
            (fill, on_fill, on_fill),
        ] {
            let painter = painter.with_clip_rect(clip.intersect(painter.clip_rect()));
            painter.galley(left_pos, left.clone(), strong);
            painter.galley(right_pos, right.clone(), normal);
        }
        resp
    }
}
