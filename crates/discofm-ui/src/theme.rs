//! Palette, widget styling and the small layout helpers shared by every panel.
use eframe::egui::{
    self, Color32, CornerRadius, FontFamily, FontId, Margin, Response, RichText, Stroke, TextStyle,
    Ui, Vec2,
};

pub(crate) const BG: Color32 = Color32::from_rgb(18, 21, 27);
pub(crate) const SURFACE: Color32 = Color32::from_rgb(25, 29, 37);
pub(crate) const RAISED: Color32 = Color32::from_rgb(34, 39, 49);
pub(crate) const INSET: Color32 = Color32::from_rgb(13, 16, 21);
pub(crate) const BORDER: Color32 = Color32::from_rgb(42, 48, 60);
pub(crate) const GRID: Color32 = Color32::from_rgb(31, 36, 45);
pub(crate) const TEXT: Color32 = Color32::from_rgb(216, 222, 232);
pub(crate) const MUTED: Color32 = Color32::from_rgb(136, 145, 162);
pub(crate) const FAINT: Color32 = Color32::from_rgb(92, 100, 116);
/// Chip output ("B") and the primary accent.
pub(crate) const ACCENT: Color32 = Color32::from_rgb(255, 155, 78);
/// Reference recording ("A").
pub(crate) const REFERENCE: Color32 = Color32::from_rgb(72, 206, 219);
pub(crate) const WARN: Color32 = Color32::from_rgb(238, 192, 92);
pub(crate) const ERROR: Color32 = Color32::from_rgb(242, 112, 112);
pub(crate) const GOOD: Color32 = Color32::from_rgb(124, 216, 153);
/// Operator identities S1–S4, also used by their envelope curves.
pub(crate) const OPERATORS: [Color32; 4] = [
    Color32::from_rgb(255, 155, 78),
    Color32::from_rgb(72, 206, 219),
    Color32::from_rgb(185, 141, 255),
    Color32::from_rgb(124, 216, 153),
];

pub(crate) fn apply(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Dark);
    // Ctrl+Plus/Minus zoom breaks the fixed panel layout at extreme scales.
    ctx.options_mut(|o| o.zoom_with_keyboard = false);
    ctx.style_mut_of(egui::Theme::Dark, |style| {
        style.text_styles = [
            (
                TextStyle::Heading,
                FontId::new(18.0, FontFamily::Proportional),
            ),
            (TextStyle::Body, FontId::new(13.5, FontFamily::Proportional)),
            (
                TextStyle::Button,
                FontId::new(13.5, FontFamily::Proportional),
            ),
            (
                TextStyle::Small,
                FontId::new(11.0, FontFamily::Proportional),
            ),
            (
                TextStyle::Monospace,
                FontId::new(12.5, FontFamily::Monospace),
            ),
        ]
        .into();
        let spacing = &mut style.spacing;
        spacing.item_spacing = Vec2::new(8.0, 6.0);
        spacing.button_padding = Vec2::new(10.0, 4.0);
        spacing.interact_size = Vec2::new(36.0, 22.0);
        spacing.combo_height = 320.0;
        spacing.tooltip_width = 340.0;
        spacing.menu_margin = Margin::same(6);
        spacing.window_margin = Margin::same(10);
        style.interaction.tooltip_delay = 0.35;

        let v = &mut style.visuals;
        v.dark_mode = true;
        v.panel_fill = BG;
        v.window_fill = SURFACE;
        v.window_stroke = Stroke::new(1.0, BORDER);
        v.window_corner_radius = CornerRadius::same(8);
        v.menu_corner_radius = CornerRadius::same(6);
        v.extreme_bg_color = INSET;
        v.faint_bg_color = Color32::from_rgb(29, 33, 42);
        v.code_bg_color = INSET;
        v.warn_fg_color = WARN;
        v.error_fg_color = ERROR;
        v.hyperlink_color = REFERENCE;
        v.selection.bg_fill = Color32::from_rgb(122, 72, 36);
        v.selection.stroke = Stroke::new(1.0, ACCENT);
        v.slider_trailing_fill = true;
        v.striped = false;
        v.indent_has_left_vline = false;
        v.collapsing_header_frame = false;

        let radius = CornerRadius::same(5);
        let w = &mut v.widgets;
        w.noninteractive.bg_fill = SURFACE;
        w.noninteractive.weak_bg_fill = SURFACE;
        w.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
        w.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
        w.noninteractive.corner_radius = radius;
        for (state, fill, stroke, text) in [
            (&mut w.inactive, RAISED, BORDER, TEXT),
            (
                &mut w.hovered,
                Color32::from_rgb(44, 50, 63),
                FAINT,
                Color32::WHITE,
            ),
            (
                &mut w.active,
                Color32::from_rgb(54, 61, 76),
                ACCENT,
                Color32::WHITE,
            ),
            (
                &mut w.open,
                Color32::from_rgb(44, 50, 63),
                FAINT,
                Color32::WHITE,
            ),
        ] {
            state.bg_fill = fill;
            state.weak_bg_fill = fill;
            state.bg_stroke = Stroke::new(1.0, stroke);
            state.fg_stroke = Stroke::new(1.5, text);
            state.corner_radius = radius;
            state.expansion = 0.0;
        }
        w.inactive.bg_stroke = Stroke::NONE;
        w.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    });
}

/// Rounded panel section with an optional coloured outline.
pub(crate) fn card() -> egui::Frame {
    egui::Frame::new()
        .fill(SURFACE)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(8)
        .inner_margin(Margin::same(12))
}

/// Small uppercase label used for section titles and control captions.
pub(crate) fn caption(text: &str) -> RichText {
    RichText::new(text.to_uppercase())
        .size(10.5)
        .extra_letter_spacing(0.6)
        .color(MUTED)
}

/// Card title row: bold title on the left, muted context on the right.
pub(crate) fn title(ui: &mut Ui, text: &str, context: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(text).size(15.0).strong().color(TEXT));
        if !context.is_empty() {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(context).small().color(FAINT));
            });
        }
    });
    ui.add_space(2.0);
}

/// A caption stacked over one control; the tooltip covers both.
pub(crate) fn field(
    ui: &mut Ui,
    label: &str,
    help: &str,
    add: impl FnOnce(&mut Ui) -> Response,
) -> Response {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.label(caption(label)).on_hover_text(help);
        add(ui).on_hover_text(help)
    })
    .inner
}

/// Mutually exclusive buttons drawn as one segmented control.
pub(crate) fn segmented<T: PartialEq + Copy>(
    ui: &mut Ui,
    value: &mut T,
    options: &[(T, &str, &str)],
) -> bool {
    let mut changed = false;
    egui::Frame::new()
        .fill(INSET)
        .corner_radius(6)
        .inner_margin(Margin::same(2))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for (option, label, help) in options {
                    let selected = *value == *option;
                    let text = if selected {
                        RichText::new(*label).color(Color32::WHITE)
                    } else {
                        RichText::new(*label).color(MUTED)
                    };
                    let mut response = ui.add(
                        egui::Button::selectable(selected, text)
                            .frame_when_inactive(false)
                            .corner_radius(5),
                    );
                    if !help.is_empty() {
                        response = response.on_hover_text(*help);
                    }
                    if response.clicked() && !selected {
                        *value = *option;
                        changed = true;
                    }
                }
            });
        });
    changed
}

/// Framed on/off button that still looks like a button while off.
pub(crate) fn toggle(ui: &mut Ui, on: &mut bool, text: &str) -> Response {
    let label = RichText::new(text).color(if *on { Color32::WHITE } else { MUTED });
    let mut response = ui.add(
        egui::Button::selectable(*on, label)
            .frame_when_inactive(true)
            .min_size(Vec2::new(30.0, 0.0)),
    );
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    response
}

/// Filled call-to-action button.
pub(crate) fn primary(text: &str) -> egui::Button<'static> {
    egui::Button::new(
        RichText::new(text.to_owned())
            .strong()
            .color(Color32::BLACK),
    )
    .fill(ACCENT)
    .min_size(Vec2::new(0.0, 28.0))
}

/// Coloured dot + label, e.g. for A/B legends.
pub(crate) fn legend(ui: &mut Ui, color: Color32, text: &str) -> Response {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(9.0), egui::Sense::hover());
        ui.painter().circle_filled(rect.center(), 4.0, color);
        ui.label(RichText::new(text).small().color(MUTED));
    })
    .response
}

/// Muted explanatory text that wraps to the available width.
pub(crate) fn note(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).small().color(FAINT));
}

/// Two-column key/value list for read-outs.
pub(crate) fn stats(ui: &mut Ui, id: &str, rows: &[(&str, String, &str)]) {
    egui::Grid::new(id)
        .num_columns(2)
        .spacing([12.0, 3.0])
        .show(ui, |ui| {
            for (key, value, help) in rows {
                let label = ui.label(RichText::new(*key).small().color(MUTED));
                let value = ui.label(RichText::new(value).monospace().color(TEXT));
                if !help.is_empty() {
                    label.on_hover_text(*help);
                    value.on_hover_text(*help);
                }
                ui.end_row();
            }
        });
}

/// Pill-shaped status badge.
pub(crate) fn badge(ui: &mut Ui, text: &str, color: Color32) -> Response {
    egui::Frame::new()
        .fill(color.gamma_multiply(0.16))
        .stroke(Stroke::new(1.0, color.gamma_multiply(0.5)))
        .corner_radius(10)
        .inner_margin(Margin::symmetric(8, 1))
        .show(ui, |ui| {
            ui.label(RichText::new(text).small().color(color));
        })
        .response
}

/// Window and taskbar icon: an FM waveform (a sine whose phase is modulated by
/// a second sine) on the surface colour, drawn at runtime so no image asset or
/// resource compiler is needed.
pub(crate) fn icon() -> egui::IconData {
    const SIZE: usize = 64;
    let mut rgba = vec![0_u8; SIZE * SIZE * 4];
    let radius = 12.0_f32;
    let half = SIZE as f32 / 2.0;
    for y in 0..SIZE {
        for x in 0..SIZE {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            // Rounded square: distance outside the inner rectangle.
            let dx = ((px - half).abs() - (half - radius)).max(0.0);
            let dy = ((py - half).abs() - (half - radius)).max(0.0);
            let coverage = (radius + 0.5 - (dx * dx + dy * dy).sqrt()).clamp(0.0, 1.0);
            let t = (px - 8.0) / (SIZE as f32 - 16.0);
            let wave = if (0.0..=1.0).contains(&t) {
                let phase = std::f32::consts::TAU
                    * (2.0 * t + 0.35 * (std::f32::consts::TAU * 3.0 * t).sin());
                let center = half - 14.0 * phase.sin();
                (3.0 - (py - center).abs()).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let mix =
                |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * wave).round() as u8;
            let i = (y * SIZE + x) * 4;
            rgba[i] = mix(SURFACE.r(), ACCENT.r());
            rgba[i + 1] = mix(SURFACE.g(), ACCENT.g());
            rgba[i + 2] = mix(SURFACE.b(), ACCENT.b());
            rgba[i + 3] = (coverage * 255.0).round() as u8;
        }
    }
    egui::IconData {
        rgba,
        width: SIZE as u32,
        height: SIZE as u32,
    }
}
