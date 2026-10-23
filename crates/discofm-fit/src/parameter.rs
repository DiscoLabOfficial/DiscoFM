use crate::{FitError, FitMode, FitStage};
use discofm_chip::{Algorithm, Frequency, Instrument, SsgEg, Value};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum OperatorField {
    Mul,
    Tl,
    Dt,
    Ar,
    D1r,
    D2r,
    Sl,
    Rr,
    Rs,
    Am,
    Ssg,
}
impl OperatorField {
    pub const ALL: [Self; 11] = [
        Self::Mul,
        Self::Tl,
        Self::Dt,
        Self::Ar,
        Self::D1r,
        Self::D2r,
        Self::Sl,
        Self::Rr,
        Self::Rs,
        Self::Am,
        Self::Ssg,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Mul => "MUL",
            Self::Tl => "TL",
            Self::Dt => "DT",
            Self::Ar => "AR",
            Self::D1r => "D1R",
            Self::D2r => "D2R",
            Self::Sl => "SL",
            Self::Rr => "RR",
            Self::Rs => "RS",
            Self::Am => "AM",
            Self::Ssg => "SSG",
        }
    }
}

/// Stable, typed parameter identities. Operator/special indices are zero based.
/// Values refer to the initial instrument of EACH run, including refinement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Parameter {
    Algorithm,
    Feedback,
    Channel3Mode,
    LfoEnabled,
    LfoRate,
    Ams,
    Fms,
    Operator { index: u8, field: OperatorField },
    SpecialFnum { index: u8 },
    SpecialBlock { index: u8 },
}
pub type ParameterLocks = BTreeSet<Parameter>;

impl Parameter {
    pub fn validate(self) -> Result<(), FitError> {
        match self {
            Self::Operator { index, .. } if index >= 4 => {
                Err(FitError::Settings("operator index must be 0..3"))
            }
            Self::SpecialFnum { index } | Self::SpecialBlock { index } if index >= 3 => {
                Err(FitError::Settings("special frequency index must be 0..2"))
            }
            _ => Ok(()),
        }
    }
    pub fn maximum(self) -> u16 {
        match self {
            Self::Algorithm
            | Self::Feedback
            | Self::LfoRate
            | Self::Fms
            | Self::SpecialBlock { .. } => 7,
            Self::LfoEnabled | Self::Channel3Mode => 1,
            Self::Ams => 3,
            Self::SpecialFnum { .. } => 2047,
            Self::Operator { field, .. } => match field {
                OperatorField::Tl => 127,
                OperatorField::Mul | OperatorField::Sl | OperatorField::Rr => 15,
                OperatorField::Ar | OperatorField::D1r | OperatorField::D2r => 31,
                OperatorField::Dt => 7,
                OperatorField::Rs => 3,
                OperatorField::Am => 1,
                OperatorField::Ssg => 8, // categorical ordinal: off, then raw 8..15
            },
        }
    }
    pub fn get(self, patch: &Instrument) -> u16 {
        match self {
            Self::Channel3Mode => patch.channel3_special.is_some().into(),
            Self::Algorithm => patch.channel.algorithm.get().into(),
            Self::Feedback => patch.channel.feedback.get().into(),
            Self::LfoEnabled => patch.global.lfo_enabled.into(),
            Self::LfoRate => patch.global.lfo_rate.get().into(),
            Self::Ams => patch.channel.ams.get().into(),
            Self::Fms => patch.channel.fms.get().into(),
            Self::Operator { index, field } => {
                let op = &patch.operators[usize::from(index)];
                match field {
                    OperatorField::Mul => op.mul.get(),
                    OperatorField::Tl => op.tl.get(),
                    OperatorField::Dt => op.dt.get(),
                    OperatorField::Ar => op.ar.get(),
                    OperatorField::D1r => op.d1r.get(),
                    OperatorField::D2r => op.d2r.get(),
                    OperatorField::Sl => op.sl.get(),
                    OperatorField::Rr => op.rr.get(),
                    OperatorField::Rs => op.rs.get(),
                    OperatorField::Am => op.am as u8,
                    OperatorField::Ssg => {
                        if op.ssg.is_enabled() {
                            op.ssg.raw() - 7
                        } else {
                            0
                        }
                    }
                }
                .into()
            }
            Self::SpecialFnum { index } => patch
                .channel3_special
                .map_or(0, |f| f[usize::from(index)].fnum()),
            Self::SpecialBlock { index } => patch
                .channel3_special
                .map_or(0, |f| u16::from(f[usize::from(index)].block())),
        }
    }
    pub(crate) fn set(self, patch: &mut Instrument, raw: u16) {
        let raw = raw.min(self.maximum());
        let byte = raw as u8;
        match self {
            Self::Channel3Mode => {
                patch.channel3_special =
                    if raw == 0 {
                        None
                    } else {
                        Some(patch.channel3_special.unwrap_or(
                            [Frequency::new(1024, 4).expect("valid default frequency"); 3],
                        ))
                    };
            }
            Self::Algorithm => {
                patch.channel.algorithm = Algorithm::new(byte).expect("projected algorithm")
            }
            Self::Feedback => {
                patch.channel.feedback = Value::new(byte).expect("projected feedback")
            }
            Self::LfoEnabled => patch.global.lfo_enabled = raw != 0,
            Self::LfoRate => patch.global.lfo_rate = Value::new(byte).expect("projected LFO"),
            Self::Ams => patch.channel.ams = Value::new(byte).expect("projected AMS"),
            Self::Fms => patch.channel.fms = Value::new(byte).expect("projected FMS"),
            Self::Operator { index, field } => {
                let op = &mut patch.operators[usize::from(index)];
                match field {
                    OperatorField::Mul => op.mul = Value::new(byte).expect("projected MUL"),
                    OperatorField::Tl => op.tl = Value::new(byte).expect("projected TL"),
                    OperatorField::Dt => op.dt = Value::new(byte).expect("projected DT"),
                    OperatorField::Ar => op.ar = Value::new(byte).expect("projected AR"),
                    OperatorField::D1r => op.d1r = Value::new(byte).expect("projected D1R"),
                    OperatorField::D2r => op.d2r = Value::new(byte).expect("projected D2R"),
                    OperatorField::Sl => op.sl = Value::new(byte).expect("projected SL"),
                    OperatorField::Rr => op.rr = Value::new(byte).expect("projected RR"),
                    OperatorField::Rs => op.rs = Value::new(byte).expect("projected RS"),
                    OperatorField::Am => op.am = raw != 0,
                    OperatorField::Ssg => {
                        op.ssg = SsgEg::try_from(if raw == 0 { 0 } else { byte + 7 })
                            .expect("projected SSG")
                    }
                }
            }
            Self::SpecialFnum { index } => {
                if let Some(f) = &mut patch.channel3_special {
                    let old = f[usize::from(index)];
                    f[usize::from(index)] =
                        Frequency::new(raw, old.block()).expect("projected FNUM");
                }
            }
            Self::SpecialBlock { index } => {
                if let Some(f) = &mut patch.channel3_special {
                    let old = f[usize::from(index)];
                    f[usize::from(index)] =
                        Frequency::new(old.fnum(), byte).expect("projected BLOCK");
                }
            }
        }
    }
    pub fn label(self) -> String {
        match self {
            Self::Channel3Mode => "Channel 3 special".into(),
            Self::Algorithm => "Algorithm".into(),
            Self::Feedback => "Feedback".into(),
            Self::LfoEnabled => "LFO on".into(),
            Self::LfoRate => "LFO rate".into(),
            Self::Ams => "AMS".into(),
            Self::Fms => "FMS".into(),
            Self::Operator { index, field } => format!("S{} {}", index + 1, field.label()),
            Self::SpecialFnum { index } => format!("S{} FNUM", index + 1),
            Self::SpecialBlock { index } => format!("S{} BLOCK", index + 1),
        }
    }
}
pub fn all_parameters(special: bool) -> Vec<Parameter> {
    let mut params = vec![
        Parameter::Algorithm,
        Parameter::Feedback,
        Parameter::Channel3Mode,
        Parameter::LfoEnabled,
        Parameter::LfoRate,
        Parameter::Ams,
        Parameter::Fms,
    ];
    for index in 0..4 {
        params.extend(OperatorField::ALL.map(|field| Parameter::Operator { index, field }));
    }
    if special {
        for index in 0..3 {
            params.extend([
                Parameter::SpecialFnum { index },
                Parameter::SpecialBlock { index },
            ]);
        }
    }
    params
}
pub(crate) fn dimensions(
    mode: FitMode,
    stage: FitStage,
    special: bool,
    locks: &ParameterLocks,
) -> Vec<Parameter> {
    all_parameters(special)
        .into_iter()
        .filter(|p| !locks.contains(p))
        .filter(|p| {
            let group = match p {
                Parameter::Algorithm
                | Parameter::Feedback
                | Parameter::Operator {
                    field: OperatorField::Mul | OperatorField::Tl | OperatorField::Dt,
                    ..
                } => FitStage::Coarse,
                Parameter::Operator {
                    field:
                        OperatorField::Ar
                        | OperatorField::D1r
                        | OperatorField::D2r
                        | OperatorField::Sl
                        | OperatorField::Rr
                        | OperatorField::Rs,
                    ..
                } => FitStage::Envelopes,
                _ => FitStage::Modulation,
            };
            if stage == FitStage::Refine {
                mode == FitMode::Advanced || group == FitStage::Coarse
            } else {
                group == stage
            }
        })
        .collect()
}
pub(crate) fn searchable(
    config: &crate::FitConfig,
    stage: FitStage,
    initial: &Instrument,
    locks: &ParameterLocks,
) -> Vec<Parameter> {
    let auto_ch3 = config.mode == FitMode::Advanced
        && config.channel3_policy == crate::ModePolicy::Auto
        && !locks.contains(&Parameter::Channel3Mode)
        && !locks.iter().any(|p| {
            matches!(
                p,
                Parameter::SpecialFnum { .. } | Parameter::SpecialBlock { .. }
            )
        });
    dimensions(
        config.mode,
        stage,
        auto_ch3
            || (initial.channel3_special.is_some()
                && (config.fit_special_frequencies
                    || config.channel3_policy == crate::ModePolicy::Auto)),
        locks,
    )
    .into_iter()
    .filter(|p| match p {
        Parameter::Channel3Mode => auto_ch3,
        Parameter::LfoEnabled | Parameter::LfoRate | Parameter::Fms => {
            config.lfo_policy == crate::ModePolicy::Auto
        }
        Parameter::Ams
        | Parameter::Operator {
            field: OperatorField::Am,
            ..
        } => config.am_policy == crate::ModePolicy::Auto,
        _ => true,
    })
    .collect()
}
pub(crate) fn key(patch: &Instrument) -> Vec<u16> {
    let mut key = all_parameters(patch.channel3_special.is_some())
        .iter()
        .map(|p| p.get(patch))
        .collect::<Vec<_>>();
    key.extend([
        patch.channel.left as u16,
        patch.channel.right as u16,
        patch.channel3_special.is_some() as u16,
    ]);
    key.extend(patch.audition.velocity_tl_depth.map(|v| u16::from(v.get())));
    key
}
