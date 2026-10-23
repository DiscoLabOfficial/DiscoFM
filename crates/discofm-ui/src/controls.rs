use crate::{
    help,
    plots::{self, Chart, Series, Unit},
    service::Preview,
    theme,
};
use discofm_chip::{Algorithm, Channel, Frequency, OperatorId, SsgEg, Value};
use discofm_formats::Session;
use eframe::egui::{self, Color32, RichText, Ui};

/// Labelled register slider; stores the raw encoding only.
fn register<const MAX: u8>(ui: &mut Ui, label: &str, help: &str, value: &mut Value<MAX>) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.add_sized(
            [34.0, 20.0],
            egui::Label::new(RichText::new(label).monospace().color(theme::MUTED)),
        )
        .on_hover_text(help);
        ui.spacing_mut().slider_width = (ui.available_width() - 44.0).max(40.0);
        let mut number = value.get();
        if ui
            .add(egui::Slider::new(&mut number, 0..=MAX).integer())
            .on_hover_text(help)
            .changed()
        {
            *value = Value::new(number).expect("bounded widget");
        }
    });
}
fn group(ui: &mut Ui, text: &str) {
    ui.add_space(4.0);
    ui.label(theme::caption(text));
}
fn describe(algorithm: Algorithm) -> String {
    let carriers = algorithm
        .routing()
        .carriers
        .iter()
        .map(|c| format!("S{}", c.number()))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "Algorithm {} (register value {}) · carriers: {carriers}",
        algorithm.display_number(),
        algorithm.get()
    )
}

pub(crate) fn instrument(ui: &mut Ui, session: &mut Session, preview: Option<&Preview>) {
    channel(ui, session);
    ui.add_space(10.0);
    let patch = &mut session.instrument;
    let algorithm = patch.channel.algorithm;
    ui.columns(4, |columns| {
        for (i, ui) in columns.iter_mut().enumerate() {
            let color = theme::OPERATORS[i];
            ui.push_id(i, |ui| {
                theme::card()
                    .stroke(egui::Stroke::new(1.0, color.gamma_multiply(0.45)))
                    .inner_margin(egui::Margin::same(10))
                    .show(ui, |ui| {
                        ui.visuals_mut().selection.bg_fill = color.gamma_multiply(0.55);
                        let op = &mut patch.operators[i];
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(format!("S{}", i + 1))
                                    .color(color)
                                    .size(20.0)
                                    .strong(),
                            );
                            let carrier = algorithm.is_carrier(OperatorId::ALL[i]);
                            theme::badge(
                                ui,
                                if carrier { "carrier" } else { "modulator" },
                                if carrier { color } else { theme::MUTED },
                            )
                            .on_hover_text(if carrier {
                                "Heard directly at the channel output."
                            } else {
                                "Modulates another operator; changes timbre, not volume."
                            });
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    let ratio = if op.mul.get() == 0 {
                                        "×½".to_owned()
                                    } else {
                                        format!("×{}", op.mul.get())
                                    };
                                    ui.label(
                                        RichText::new(format!(
                                            "{ratio}  DT {:+}",
                                            op.signed_detune()
                                        ))
                                        .monospace()
                                        .color(theme::MUTED),
                                    )
                                    .on_hover_text(
                                        "Effective frequency multiple and signed detune.",
                                    );
                                },
                            );
                        });
                        group(ui, "Level · pitch");
                        register(ui, "TL", help::TL, &mut op.tl);
                        register(ui, "MUL", help::MUL, &mut op.mul);
                        register(ui, "DT", help::DT, &mut op.dt);
                        group(ui, "Envelope");
                        register(ui, "AR", help::AR, &mut op.ar);
                        register(ui, "D1R", help::D1R, &mut op.d1r);
                        register(ui, "SL", help::SL, &mut op.sl);
                        register(ui, "D2R", help::D2R, &mut op.d2r);
                        register(ui, "RR", help::RR, &mut op.rr);
                        register(ui, "RS", help::RS, &mut op.rs);
                        register(
                            ui,
                            "VEL",
                            help::VEL,
                            &mut patch.audition.velocity_tl_depth[i],
                        );
                        ui.add_space(2.0);
                        ui.horizontal(|ui| {
                            theme::toggle(ui, &mut op.am, "AM").on_hover_text(help::AM);
                            egui::ComboBox::from_id_salt("ssg")
                                .width((ui.available_width() - 4.0).max(80.0))
                                .selected_text(ssg_label(op.ssg, false))
                                .show_ui(ui, |ui| {
                                    for ssg in SsgEg::ALL {
                                        ui.selectable_value(&mut op.ssg, ssg, ssg_label(ssg, true));
                                    }
                                })
                                .response
                                .on_hover_text(help::SSG);
                        });
                        ui.add_space(4.0);
                        let empty: [Vec<[f64; 2]>; 2] = Default::default();
                        let curves = preview.map_or(&empty, |p| &p.curves.operators[i]);
                        Chart::new(86.0, [0.0, session.audition.total_seconds], [1023.0, 0.0])
                            .units(Unit::Seconds, Unit::Attenuation)
                            .show(
                                ui,
                                &[
                                    Series::line("raw", &curves[0], Color32::from_gray(105)),
                                    Series::area("out", &curves[1], color),
                                ],
                            )
                            .on_hover_text(help::EG_PLOT);
                        if let Some(v) = preview.and_then(|p| p.render.envelopes.last()) {
                            theme::note(ui, &format!("Ends in {:?}", v.operators[i].stage));
                        }
                    });
            });
        }
    });
}
fn ssg_label(ssg: SsgEg, long: bool) -> String {
    if !ssg.is_enabled() {
        return "SSG-EG off".into();
    }
    if long {
        let flag = |on: bool, name: &str| {
            if on {
                name.to_owned()
            } else {
                "·".repeat(name.len())
            }
        };
        format!(
            "SSG {} · {} {} {}",
            ssg.raw(),
            flag(ssg.inverted(), "INV"),
            flag(ssg.alternate(), "ALT"),
            flag(ssg.hold(), "HOLD")
        )
    } else {
        format!("SSG-EG {}", ssg.raw())
    }
}

fn channel(ui: &mut Ui, session: &mut Session) {
    let patch = &mut session.instrument;
    theme::card().show(ui, |ui| {
        ui.columns(3, |columns| {
            let ui = &mut columns[0];
            ui.label(theme::caption("Algorithm"))
                .on_hover_text(help::ALGORITHM);
            plots::algorithm(
                ui,
                patch.channel.algorithm,
                patch.channel.feedback.get(),
                120.0,
            )
            .on_hover_text(describe(patch.channel.algorithm));
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 3.0;
                for algorithm in Algorithm::ALL {
                    let selected = patch.channel.algorithm == algorithm;
                    let response = ui
                        .add(
                            egui::Button::selectable(
                                selected,
                                RichText::new(algorithm.display_number().to_string()).monospace(),
                            )
                            .min_size(egui::vec2(24.0, 22.0)),
                        )
                        .on_hover_ui(|ui| {
                            ui.label(describe(algorithm));
                            plots::algorithm(ui, algorithm, patch.channel.feedback.get(), 64.0);
                        });
                    if response.clicked() {
                        patch.channel.algorithm = algorithm;
                    }
                }
            });

            let ui = &mut columns[1];
            ui.visuals_mut().selection.bg_fill = theme::ACCENT.gamma_multiply(0.55);
            ui.label(theme::caption("Operator 1 feedback"));
            register(ui, "FB", help::FEEDBACK, &mut patch.channel.feedback);
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(theme::caption("LFO")).on_hover_text(help::LFO);
                let label = if patch.global.lfo_enabled {
                    "On"
                } else {
                    "Off"
                };
                theme::toggle(ui, &mut patch.global.lfo_enabled, label).on_hover_text(help::LFO);
            });
            ui.add_enabled_ui(patch.global.lfo_enabled, |ui| {
                register(ui, "RATE", help::LFO_RATE, &mut patch.global.lfo_rate);
            });
            register(ui, "AMS", help::AMS, &mut patch.channel.ams);
            register(ui, "FMS", help::FMS, &mut patch.channel.fms);

            let ui = &mut columns[2];
            ui.label(theme::caption("Output"));
            ui.horizontal(|ui| {
                theme::toggle(ui, &mut patch.channel.left, "L").on_hover_text(help::STEREO);
                theme::toggle(ui, &mut patch.channel.right, "R").on_hover_text(help::STEREO);
                if !patch.channel.left && !patch.channel.right {
                    ui.label(RichText::new("muted").small().color(theme::WARN));
                }
            });
            ui.add_space(6.0);
            let mut special = patch.channel3_special.is_some();
            if ui
                .checkbox(&mut special, "Channel 3 special mode")
                .on_hover_text(help::CH3)
                .changed()
            {
                patch.channel3_special = if special {
                    session.audition.channel = Channel::THREE;
                    Some([Frequency::new(1000, 3).expect("constant"); 3])
                } else {
                    None
                };
            }
            if let Some(frequencies) = &mut patch.channel3_special {
                egui::Grid::new("special-frequencies")
                    .num_columns(3)
                    .spacing([8.0, 4.0])
                    .show(ui, |ui| {
                        for (i, frequency) in frequencies.iter_mut().enumerate() {
                            ui.label(
                                RichText::new(format!("S{}", i + 1))
                                    .color(theme::OPERATORS[i])
                                    .monospace(),
                            );
                            let (mut fnum, mut block) = (frequency.fnum(), frequency.block());
                            ui.add(
                                egui::DragValue::new(&mut fnum)
                                    .range(0..=2047)
                                    .prefix("FNUM "),
                            )
                            .on_hover_text(help::FNUM);
                            ui.add(egui::DragValue::new(&mut block).range(0..=7).prefix("BLK "))
                                .on_hover_text(help::BLOCK);
                            *frequency = Frequency::new(fnum, block).expect("bounded widget");
                            ui.end_row();
                        }
                    });
                theme::note(ui, "S4 plays the audition note.");
            } else {
                theme::note(ui, "All operators follow the audition note.");
            }
        });
    });
}
