//! Lightweight painters for curves, spectra, spectrograms and algorithm graphs.
use crate::theme;
use discofm_analysis::{Analysis, Spectrogram};
use discofm_audio::AudioBuffer;
use discofm_chip::{Algorithm, OperatorId};
use eframe::egui::{
    self, Align2, Color32, FontId, Mesh, Pos2, Rect, Response, Sense, Shape, Stroke, Ui, Vec2,
    epaint::CubicBezierShape,
};

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Unit {
    Seconds,
    Hz,
    LogHz,
    Db,
    Level,
    Attenuation,
    Count,
    Loss,
}
impl Unit {
    pub(crate) fn format(self, v: f64) -> String {
        match self {
            Self::Seconds if v.abs() < 1.0 => format!("{:.0} ms", v * 1000.0),
            Self::Seconds => format!("{v:.2} s"),
            Self::Hz => hz(v),
            Self::LogHz => hz(10f64.powf(v)),
            Self::Db => format!("{v:.1} dB"),
            Self::Level => format!("{v:.3}"),
            Self::Attenuation | Self::Count => format!("{v:.0}"),
            Self::Loss => format!("{v:.4}"),
        }
    }
}
fn hz(v: f64) -> String {
    if v >= 1000.0 {
        format!("{:.2} kHz", v / 1000.0)
    } else {
        format!("{v:.0} Hz")
    }
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Kind {
    Line,
    /// Line with a translucent fill down to the bottom of the chart.
    Area,
    /// Consecutive (low, high) pairs sharing one x, as produced by [`waveform`].
    MinMax,
}
pub(crate) struct Series<'a> {
    pub name: &'a str,
    pub points: &'a [[f64; 2]],
    pub color: Color32,
    pub kind: Kind,
}
impl<'a> Series<'a> {
    pub fn line(name: &'a str, points: &'a [[f64; 2]], color: Color32) -> Self {
        Self {
            name,
            points,
            color,
            kind: Kind::Line,
        }
    }
    pub fn area(name: &'a str, points: &'a [[f64; 2]], color: Color32) -> Self {
        Self {
            kind: Kind::Area,
            ..Self::line(name, points, color)
        }
    }
    pub fn min_max(name: &'a str, points: &'a [[f64; 2]], color: Color32) -> Self {
        Self {
            kind: Kind::MinMax,
            ..Self::line(name, points, color)
        }
    }
}

pub(crate) struct Chart {
    height: f32,
    x: [f64; 2],
    y: [f64; 2],
    x_unit: Unit,
    y_unit: Unit,
}
impl Chart {
    pub fn new(height: f32, x: [f64; 2], y: [f64; 2]) -> Self {
        Self {
            height,
            x,
            y,
            x_unit: Unit::Seconds,
            y_unit: Unit::Level,
        }
    }
    pub fn units(mut self, x: Unit, y: Unit) -> Self {
        self.x_unit = x;
        self.y_unit = y;
        self
    }
    pub fn show(&self, ui: &mut Ui, series: &[Series<'_>]) -> Response {
        let (response, painter) =
            ui.allocate_painter(Vec2::new(ui.available_width(), self.height), Sense::hover());
        let outer = response.rect;
        painter.rect_filled(outer, 6, theme::INSET);
        let rect = Rect::from_min_max(
            outer.min + Vec2::new(8.0, 8.0),
            outer.max - Vec2::new(8.0, 18.0),
        );
        let span = |r: [f64; 2]| {
            if (r[1] - r[0]).abs() < 1e-12 {
                1.0
            } else {
                r[1] - r[0]
            }
        };
        let (xs, ys) = (span(self.x), span(self.y));
        let to_x =
            |v: f64| rect.left() + ((v - self.x[0]) / xs).clamp(0.0, 1.0) as f32 * rect.width();
        let to_y =
            |v: f64| rect.bottom() - ((v - self.y[0]) / ys).clamp(0.0, 1.0) as f32 * rect.height();
        let grid = Stroke::new(1.0, theme::GRID);
        for n in 0..=4 {
            let y = egui::lerp(rect.y_range(), n as f32 / 4.0);
            painter.hline(rect.x_range(), y, grid);
        }
        let font = FontId::monospace(9.5);
        for tick in ticks(self.x, self.x_unit) {
            let x = to_x(tick);
            painter.vline(x, rect.y_range(), grid);
            painter.text(
                Pos2::new(x, outer.bottom() - 4.0),
                Align2::CENTER_BOTTOM,
                self.x_unit.format(tick),
                font.clone(),
                theme::FAINT,
            );
        }
        for (value, anchor, y) in [
            (self.y[1], Align2::LEFT_TOP, rect.top() + 1.0),
            (self.y[0], Align2::LEFT_BOTTOM, rect.bottom() - 1.0),
        ] {
            painter.text(
                Pos2::new(rect.left() + 2.0, y),
                anchor,
                self.y_unit.format(value),
                font.clone(),
                theme::FAINT,
            );
        }
        let painter = painter.with_clip_rect(rect.expand(1.0));
        for s in series {
            let finite = |p: &&[f64; 2]| p[0].is_finite() && p[1].is_finite();
            match s.kind {
                Kind::MinMax => {
                    let stroke = Stroke::new(1.0, s.color);
                    for pair in s.points.as_chunks::<2>().0 {
                        if pair.iter().all(|p| finite(&p)) {
                            let x = to_x(pair[0][0]);
                            let (a, b) = (to_y(pair[0][1]), to_y(pair[1][1]));
                            painter.vline(
                                x,
                                egui::Rangef::new(a.min(b) - 0.5, a.max(b) + 0.5),
                                stroke,
                            );
                        }
                    }
                }
                Kind::Line | Kind::Area => {
                    let points = s
                        .points
                        .iter()
                        .filter(finite)
                        .map(|p| Pos2::new(to_x(p[0]), to_y(p[1])))
                        .collect::<Vec<_>>();
                    if points.len() < 2 {
                        continue;
                    }
                    if s.kind == Kind::Area {
                        let fill = s.color.gamma_multiply(0.14);
                        let mut mesh = Mesh::default();
                        for w in points.windows(2) {
                            let base = mesh.vertices.len() as u32;
                            mesh.colored_vertex(w[0], fill);
                            mesh.colored_vertex(w[1], fill);
                            mesh.colored_vertex(Pos2::new(w[1].x, rect.bottom()), fill);
                            mesh.colored_vertex(Pos2::new(w[0].x, rect.bottom()), fill);
                            mesh.add_triangle(base, base + 1, base + 2);
                            mesh.add_triangle(base, base + 2, base + 3);
                        }
                        painter.add(Shape::mesh(mesh));
                    }
                    painter.add(Shape::line(points, Stroke::new(1.4, s.color)));
                }
            }
        }
        if let Some(pos) = response.hover_pos().filter(|p| rect.contains(*p)) {
            let x = self.x[0] + f64::from((pos.x - rect.left()) / rect.width()) * xs;
            painter.vline(pos.x, rect.y_range(), Stroke::new(1.0, theme::FAINT));
            let mut lines = vec![(theme::MUTED, self.x_unit.format(x))];
            for s in series.iter().filter(|s| !s.points.is_empty()) {
                let i = s
                    .points
                    .partition_point(|p| p[0] < x)
                    .min(s.points.len() - 1);
                let i = if i > 0 && (s.points[i - 1][0] - x).abs() < (s.points[i][0] - x).abs() {
                    i - 1
                } else {
                    i
                };
                let text = if s.kind == Kind::MinMax {
                    let j = i & !1;
                    let pair = &s.points[j..(j + 2).min(s.points.len())];
                    let (lo, hi) = (pair[0][1], pair[pair.len() - 1][1]);
                    format!(
                        "{}  {} … {}",
                        s.name,
                        self.y_unit.format(lo),
                        self.y_unit.format(hi)
                    )
                } else {
                    let p = s.points[i];
                    if p[1].is_finite() {
                        painter.circle_filled(Pos2::new(to_x(p[0]), to_y(p[1])), 3.0, s.color);
                    }
                    format!("{}  {}", s.name, self.y_unit.format(p[1]))
                };
                lines.push((s.color, text));
            }
            readout(&painter, outer, pos, &lines);
        }
        response
    }
}

/// Small floating label box next to the pointer, kept inside `bounds`.
fn readout(painter: &egui::Painter, bounds: Rect, pos: Pos2, lines: &[(Color32, String)]) {
    let font = FontId::monospace(10.5);
    let galleys = lines
        .iter()
        .map(|(c, t)| painter.layout_no_wrap(t.clone(), font.clone(), *c))
        .collect::<Vec<_>>();
    let width = galleys.iter().map(|g| g.size().x).fold(0.0, f32::max) + 12.0;
    let height = galleys.iter().map(|g| g.size().y).sum::<f32>() + 8.0;
    let mut min = pos + Vec2::new(12.0, -height - 6.0);
    if min.x + width > bounds.right() {
        min.x = pos.x - width - 12.0;
    }
    min.y = min
        .y
        .clamp(bounds.top(), (bounds.bottom() - height).max(bounds.top()));
    let frame = Rect::from_min_size(min, Vec2::new(width, height));
    let painter = painter.with_clip_rect(bounds);
    painter.rect(
        frame,
        4,
        theme::SURFACE.gamma_multiply(0.95),
        Stroke::new(1.0, theme::BORDER),
        egui::StrokeKind::Inside,
    );
    let mut y = frame.top() + 4.0;
    for g in galleys {
        let h = g.size().y;
        painter.galley(Pos2::new(frame.left() + 6.0, y), g, theme::TEXT);
        y += h;
    }
}

fn ticks(range: [f64; 2], unit: Unit) -> Vec<f64> {
    let (lo, hi) = (range[0].min(range[1]), range[0].max(range[1]));
    if !lo.is_finite() || !hi.is_finite() || hi <= lo {
        return Vec::new();
    }
    if unit == Unit::LogHz {
        return [50.0, 100.0, 200.0, 500.0, 1e3, 2e3, 5e3, 1e4, 2e4]
            .map(f64::log10)
            .into_iter()
            .filter(|v| *v > lo && *v < hi)
            .collect();
    }
    let raw = (hi - lo) / 5.0;
    let magnitude = 10f64.powf(raw.log10().floor());
    let step = [1.0, 2.0, 5.0, 10.0]
        .into_iter()
        .map(|m| m * magnitude)
        .find(|s| *s >= raw)
        .unwrap_or(10.0 * magnitude);
    let mut out = Vec::new();
    let mut v = (lo / step).floor() * step + step;
    while v < hi - step * 0.25 && out.len() < 12 {
        out.push(v);
        v += step;
    }
    out
}

/// Min/max pairs per display column; the input buffer is never modified.
pub(crate) fn waveform(audio: &AudioBuffer) -> Vec<[f64; 2]> {
    let mut points = Vec::new();
    let chunk = audio.frames.len().div_ceil(800).max(1);
    for (n, frames) in audio.frames.chunks(chunk).enumerate() {
        let (mut low, mut high) = (f32::INFINITY, f32::NEG_INFINITY);
        for frame in frames {
            for v in frame {
                low = low.min(*v);
                high = high.max(*v);
            }
        }
        let t = (n * chunk) as f64 / f64::from(audio.sample_rate);
        points.push([t, f64::from(low)]);
        points.push([t, f64::from(high)]);
    }
    points
}

const HEAT: [(f32, [f32; 3]); 6] = [
    (0.0, [13.0, 16.0, 21.0]),
    (0.22, [34.0, 30.0, 82.0]),
    (0.45, [122.0, 38.0, 112.0]),
    (0.68, [222.0, 84.0, 62.0]),
    (0.86, [252.0, 164.0, 58.0]),
    (1.0, [252.0, 236.0, 164.0]),
];
fn heat(v: f32) -> Color32 {
    let i = HEAT
        .iter()
        .position(|(s, _)| *s >= v)
        .unwrap_or(HEAT.len() - 1)
        .max(1);
    let ((a, ca), (b, cb)) = (HEAT[i - 1], HEAT[i]);
    let t = ((v - a) / (b - a)).clamp(0.0, 1.0);
    let c = |k: usize| (ca[k] + (cb[k] - ca[k]) * t) as u8;
    Color32::from_rgb(c(0), c(1), c(2))
}
pub(crate) fn texture(ctx: &egui::Context, name: &str, gram: &Spectrogram) -> egui::TextureHandle {
    let (width, height) = (gram.times.len(), gram.frequencies.len());
    let pixels = (0..height)
        .flat_map(|row| {
            (0..width).map(move |column| {
                let db = gram.db[column * height + (height - 1 - row)];
                heat(((db + 100.0) / 100.0).clamp(0.0, 1.0))
            })
        })
        .collect();
    ctx.load_texture(
        name,
        egui::ColorImage::new([width, height], pixels),
        egui::TextureOptions::LINEAR,
    )
}
pub(crate) fn spectrogram(
    ui: &mut Ui,
    texture: Option<&egui::TextureHandle>,
    gram: Option<&Spectrogram>,
    seconds: f64,
    rate: u32,
) {
    let size = Vec2::new(ui.available_width(), 130.0);
    let Some(texture) = texture else {
        let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
        ui.painter().rect_filled(rect, 6, theme::INSET);
        return;
    };
    let response = ui.add(
        egui::Image::new(texture)
            .fit_to_exact_size(size)
            .maintain_aspect_ratio(false)
            .corner_radius(6)
            .sense(Sense::hover()),
    );
    let rect = response.rect;
    let painter = ui.painter_at(rect);
    let font = FontId::monospace(9.5);
    let shadow = Color32::from_black_alpha(170);
    for (text, pos, align) in [
        (
            "0".to_owned(),
            rect.left_bottom() + Vec2::new(4.0, -3.0),
            Align2::LEFT_BOTTOM,
        ),
        (
            Unit::Seconds.format(seconds),
            rect.right_bottom() + Vec2::new(-4.0, -3.0),
            Align2::RIGHT_BOTTOM,
        ),
        (
            hz(gram
                .and_then(|g| g.frequencies.last())
                .map_or(f64::from(rate) / 2.0, |f| f64::from(*f))),
            rect.right_top() + Vec2::new(-4.0, 3.0),
            Align2::RIGHT_TOP,
        ),
    ] {
        painter.text(pos + Vec2::splat(1.0), align, &text, font.clone(), shadow);
        painter.text(pos, align, text, font.clone(), theme::TEXT);
    }
    if let (Some(pos), Some(gram)) = (response.hover_pos(), gram)
        && !gram.times.is_empty()
        && !gram.frequencies.is_empty()
    {
        let (w, h) = (gram.times.len(), gram.frequencies.len());
        let column =
            (((pos.x - rect.left()) / rect.width()) * w as f32).clamp(0.0, w as f32 - 1.0) as usize;
        let row = (((rect.bottom() - pos.y) / rect.height()) * h as f32).clamp(0.0, h as f32 - 1.0)
            as usize;
        let t = (pos.x - rect.left()) / rect.width() * seconds as f32;
        painter.vline(
            pos.x,
            rect.y_range(),
            Stroke::new(1.0, Color32::from_white_alpha(60)),
        );
        painter.hline(
            rect.x_range(),
            pos.y,
            Stroke::new(1.0, Color32::from_white_alpha(60)),
        );
        readout(
            &painter,
            rect,
            pos,
            &[
                (theme::MUTED, Unit::Seconds.format(f64::from(t))),
                (theme::TEXT, hz(f64::from(gram.frequencies[row]))),
                (
                    theme::ACCENT,
                    format!("{:.1} dBFS", gram.db[column * h + row]),
                ),
            ],
        );
    }
}
/// Plot-ready series derived once per analysis on the preview worker, so
/// frames only borrow them.
#[derive(Default)]
pub(crate) struct Curves {
    pub rms: Vec<[f64; 2]>,
    pub peak: Vec<[f64; 2]>,
    pub centroid: Vec<[f64; 2]>,
    pub flux: Vec<[f64; 2]>,
    /// log10(Hz) against dBFS, from 20 Hz.
    pub spectrum: Vec<[f64; 2]>,
    pub sample_rate: u32,
}
impl Curves {
    pub fn new(a: &Analysis) -> Self {
        Self {
            rms: a
                .envelope
                .iter()
                .map(|v| [v.seconds, f64::from(v.rms)])
                .collect(),
            peak: a
                .envelope
                .iter()
                .map(|v| [v.seconds, f64::from(v.peak)])
                .collect(),
            centroid: a
                .spectral
                .iter()
                .map(|v| [v.seconds, f64::from(v.centroid_hz)])
                .collect(),
            flux: a
                .spectral
                .iter()
                .map(|v| [v.seconds, f64::from(v.flux)])
                .collect(),
            spectrum: a
                .spectrum
                .frequencies
                .iter()
                .zip(&a.spectrum.magnitudes)
                .filter(|(f, _)| **f >= 20.0)
                .map(|(f, m)| {
                    [
                        f64::from(*f).log10(),
                        f64::from(*m).max(1e-6).log10() * 20.0,
                    ]
                })
                .collect(),
            sample_rate: a.sample_rate,
        }
    }
}
pub(crate) fn max_y(series: &[&[[f64; 2]]], floor: f64) -> f64 {
    series
        .iter()
        .flat_map(|s| s.iter())
        .map(|p| p[1])
        .filter(|v| v.is_finite())
        .fold(floor, f64::max)
}
pub(crate) fn spectrum(ui: &mut Ui, a: Option<&Curves>, b: Option<&Curves>) {
    let maximum = a
        .map_or(48000, |a| a.sample_rate)
        .max(b.map_or(48000, |b| b.sample_rate))
        / 2;
    Chart::new(
        220.0,
        [20f64.log10(), f64::from(maximum).log10()],
        [-100.0, 0.0],
    )
    .units(Unit::LogHz, Unit::Db)
    .show(
        ui,
        &[
            Series::line("A", a.map_or(&[][..], |c| &c.spectrum), theme::REFERENCE),
            Series::line("B", b.map_or(&[][..], |c| &c.spectrum), theme::ACCENT),
        ],
    );
}

/// Operator routing laid out by distance from the output: left→right for
/// chains, top→bottom when several operators share a level.
pub(crate) fn algorithm(ui: &mut Ui, algorithm: Algorithm, feedback: u8, height: f32) -> Response {
    let width = ui.available_width().min(height * 3.4);
    let (response, painter) = ui.allocate_painter(Vec2::new(width, height), Sense::hover());
    let outer = response.rect;
    painter.rect_filled(outer, 6, theme::INSET);
    let radius = (height * 0.11).clamp(6.0, 13.0);
    let routing = algorithm.routing();
    let mut depth = [0usize; 4];
    for _ in 0..4 {
        for (from, to) in routing.connections {
            depth[from.index()] = depth[from.index()].max(depth[to.index()] + 1);
        }
    }
    let deepest = depth.iter().copied().max().unwrap_or(0);
    let widest = (0..=deepest)
        .map(|d| depth.iter().filter(|v| **v == d).count())
        .max()
        .unwrap_or(1);
    let across = widest <= 2;
    let bus_gap = radius * 1.7;
    let area = if across {
        Rect::from_min_max(
            outer.min + Vec2::new(radius * 0.8, radius * 2.6),
            outer.max - Vec2::new(bus_gap + radius * 1.4, radius * 0.6),
        )
    } else {
        Rect::from_min_max(
            outer.min + Vec2::new(radius * 3.0, radius * 2.6),
            outer.max - Vec2::new(radius * 3.0, bus_gap + radius * 0.6),
        )
    };
    let mut nodes = [Pos2::ZERO; 4];
    for d in 0..=deepest {
        let level = (deepest - d) as f32 + 0.5;
        let ids = OperatorId::ALL
            .into_iter()
            .filter(|id| depth[id.index()] == d)
            .collect::<Vec<_>>();
        for (n, id) in ids.iter().enumerate() {
            let along = (n as f32 + 0.5) / ids.len() as f32;
            let level = level / (deepest as f32 + 1.0);
            nodes[id.index()] = if across {
                Pos2::new(
                    egui::lerp(area.x_range(), level),
                    egui::lerp(area.y_range(), along),
                )
            } else {
                Pos2::new(
                    egui::lerp(area.x_range(), along),
                    egui::lerp(area.y_range(), level),
                )
            };
        }
    }
    let wire = Stroke::new(1.4, theme::MUTED);
    for (from, to) in routing.connections {
        let (a, b) = (nodes[from.index()], nodes[to.index()]);
        let dir = (b - a).normalized();
        arrow(
            &painter,
            a + dir * radius,
            b - dir * (radius + 1.0),
            wire,
            radius * 0.5,
        );
    }
    let bus_stroke = Stroke::new(2.0, theme::ACCENT.gamma_multiply(0.7));
    let carriers = routing.carriers.iter().map(|c| nodes[c.index()]);
    let (lo, hi) = carriers.fold(
        (Pos2::new(f32::MAX, f32::MAX), Pos2::new(f32::MIN, f32::MIN)),
        |(lo, hi), p| (lo.min(p), hi.max(p)),
    );
    let label_at = if across {
        let bus = area.right() + bus_gap;
        painter.vline(
            bus,
            egui::Rangef::new(lo.y - radius * 0.6, hi.y + radius * 0.6),
            bus_stroke,
        );
        for c in routing.carriers {
            let p = nodes[c.index()];
            arrow(
                &painter,
                p + Vec2::new(radius, 0.0),
                Pos2::new(bus - 1.0, p.y),
                wire,
                radius * 0.5,
            );
        }
        (Pos2::new(bus, lo.y - radius * 0.9), Align2::CENTER_BOTTOM)
    } else {
        let bus = area.bottom() + bus_gap;
        painter.hline(
            egui::Rangef::new(lo.x - radius * 0.6, hi.x + radius * 0.6),
            bus,
            bus_stroke,
        );
        for c in routing.carriers {
            let p = nodes[c.index()];
            arrow(
                &painter,
                p + Vec2::new(0.0, radius),
                Pos2::new(p.x, bus - 1.0),
                wire,
                radius * 0.5,
            );
        }
        (Pos2::new(hi.x + radius * 1.0, bus), Align2::LEFT_CENTER)
    };
    if height >= 70.0 {
        painter.text(
            label_at.0,
            label_at.1,
            "OUT",
            FontId::monospace(9.0),
            theme::ACCENT,
        );
    }
    let s1 = nodes[0];
    let fb = theme::OPERATORS[0].gamma_multiply(if feedback == 0 { 0.35 } else { 1.0 });
    painter.add(CubicBezierShape::from_points_stroke(
        [
            s1 + Vec2::new(radius * 0.45, -radius * 0.9),
            s1 + Vec2::new(radius * 1.6, -radius * 2.7),
            s1 + Vec2::new(-radius * 1.6, -radius * 2.7),
            s1 + Vec2::new(-radius * 0.45, -radius * 0.95),
        ],
        false,
        Color32::TRANSPARENT,
        Stroke::new(1.2, fb),
    ));
    if height >= 70.0 {
        painter.text(
            s1 + Vec2::new(radius * 1.3, -radius * 2.1),
            Align2::LEFT_CENTER,
            format!("FB {feedback}"),
            FontId::monospace(9.5),
            fb,
        );
    }
    for id in OperatorId::ALL {
        let p = nodes[id.index()];
        let color = theme::OPERATORS[id.index()];
        painter.circle(
            p,
            radius,
            color.gamma_multiply(0.18),
            Stroke::new(1.5, color),
        );
        painter.text(
            p,
            Align2::CENTER_CENTER,
            id.number().to_string(),
            FontId::monospace(radius.max(8.0)),
            color,
        );
    }
    response
}
fn arrow(painter: &egui::Painter, from: Pos2, to: Pos2, stroke: Stroke, head: f32) {
    let dir = (to - from).normalized();
    if !dir.is_finite() {
        return;
    }
    painter.line_segment([from, to - dir * head * 0.8], stroke);
    let normal = Vec2::new(-dir.y, dir.x) * head * 0.55;
    painter.add(Shape::convex_polygon(
        vec![to, to - dir * head + normal, to - dir * head - normal],
        stroke.color,
        Stroke::NONE,
    ));
}
