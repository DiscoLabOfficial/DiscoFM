//! Tracker instrument files as written by GENNY (superjoebob/genny) and read by
//! TFM Music Maker, VGM Music Maker, Deflemask and Furnace. All three store raw
//! register values with operators in register order (S1, S3, S2, S4), not in
//! datasheet order.
use crate::InstrumentExporter;
use discofm_chip::{
    Algorithm, ChannelParameters, Instrument, Operator, OperatorId, ParameterError, SsgEg, Value,
};
use std::path::Path;
use thiserror::Error;

const FILE_ORDER: [OperatorId; 4] = [
    OperatorId::S1,
    OperatorId::S3,
    OperatorId::S2,
    OperatorId::S4,
];
/// TFI/VGI detune is 0..6 centred on 3; the register keeps sign in bit 2, so
/// raw 4 (negative zero) is written as plain zero, which the chip treats alike.
const DT_TO_FILE: [u8; 8] = [3, 4, 5, 6, 3, 2, 1, 0];
const DT_FROM_FILE: [u8; 7] = [7, 6, 5, 0, 1, 2, 3];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InstrumentFormat {
    /// TFM Music Maker, 42 bytes: no LFO sensitivity, AM or panning.
    Tfi,
    /// VGM Music Maker, 43 bytes: TFI plus AMS/FMS and AM.
    Vgi,
    /// Register dump with a "YI" trailer, 32 bytes: VGI plus panning.
    Tyi,
}

#[derive(Debug, Error)]
pub enum ImportError {
    #[error("{format} files are {expected} bytes, got {actual}")]
    Size {
        format: &'static str,
        expected: usize,
        actual: usize,
    },
    #[error("TYI file lacks the \"YI\" trailer")]
    Trailer,
    #[error("unknown instrument file extension (expected .tfi, .vgi or .tyi)")]
    Extension,
    #[error("{field}: {source}")]
    Value {
        field: String,
        #[source]
        source: ParameterError,
    },
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl InstrumentFormat {
    pub const ALL: [Self; 3] = [Self::Tfi, Self::Vgi, Self::Tyi];

    pub const fn extension(self) -> &'static str {
        match self {
            Self::Tfi => "tfi",
            Self::Vgi => "vgi",
            Self::Tyi => "tyi",
        }
    }
    pub const fn name(self) -> &'static str {
        match self {
            Self::Tfi => "TFM Music Maker",
            Self::Vgi => "VGM Music Maker",
            Self::Tyi => "TYI register dump",
        }
    }
    pub const fn size(self) -> usize {
        match self {
            Self::Tfi => 42,
            Self::Vgi => 43,
            Self::Tyi => 32,
        }
    }
    pub fn from_path(path: &Path) -> Option<Self> {
        let extension = path.extension()?.to_str()?;
        Self::ALL
            .into_iter()
            .find(|format| format.extension().eq_ignore_ascii_case(extension))
    }

    /// Parts of the instrument this format cannot store, in editor wording.
    /// Empty means the file reloads into an identical instrument (apart from
    /// negative-zero detune, which sounds the same).
    pub fn omitted(self, instrument: &Instrument) -> Vec<&'static str> {
        let channel = &instrument.channel;
        let mut omitted = Vec::new();
        if self == Self::Tfi {
            if instrument.operators.iter().any(|op| op.am) {
                omitted.push("operator AM enable");
            }
            if channel.ams.get() != 0 || channel.fms.get() != 0 {
                omitted.push("AMS/FMS sensitivity");
            }
        }
        let pan_lost = match self {
            Self::Tyi => !channel.left && !channel.right,
            _ => !(channel.left && channel.right),
        };
        if pan_lost {
            omitted.push("panning");
        }
        if instrument.global.lfo_enabled || instrument.global.lfo_rate.get() != 0 {
            omitted.push("global LFO enable/rate");
        }
        if instrument.channel3_special.is_some() {
            omitted.push("channel 3 special frequencies");
        }
        if instrument
            .audition
            .velocity_tl_depth
            .iter()
            .any(|depth| depth.get() != 0)
        {
            omitted.push("velocity depth");
        }
        omitted
    }

    pub fn encode(self, instrument: &Instrument) -> Vec<u8> {
        let channel = &instrument.channel;
        let ops = FILE_ORDER.map(|id| *instrument.operator(id));
        let ams_fms = (channel.ams.get() << 4) | channel.fms.get();
        let mut bytes = Vec::with_capacity(self.size());
        match self {
            Self::Tfi | Self::Vgi => {
                bytes.extend([channel.algorithm.get(), channel.feedback.get()]);
                if self == Self::Vgi {
                    bytes.push(ams_fms);
                }
                for op in ops {
                    let d1r = if self == Self::Vgi {
                        (u8::from(op.am) << 7) | op.d1r.get()
                    } else {
                        op.d1r.get()
                    };
                    bytes.extend([
                        op.mul.get(),
                        DT_TO_FILE[op.dt.get() as usize],
                        op.tl.get(),
                        op.rs.get(),
                        op.ar.get(),
                        d1r,
                        op.d2r.get(),
                        op.rr.get(),
                        op.sl.get(),
                        op.ssg.raw(),
                    ]);
                }
            }
            Self::Tyi => {
                let fields: [fn(&Operator) -> u8; 7] = [
                    |op| (op.dt.get() << 4) | op.mul.get(),
                    |op| op.tl.get(),
                    |op| (op.rs.get() << 6) | op.ar.get(),
                    |op| (u8::from(op.am) << 7) | op.d1r.get(),
                    |op| op.d2r.get(),
                    |op| (op.sl.get() << 4) | op.rr.get(),
                    |op| op.ssg.raw(),
                ];
                for field in fields {
                    bytes.extend(ops.iter().map(field));
                }
                bytes.push((channel.feedback.get() << 3) | channel.algorithm.get());
                // GENNY leaves the pan bits clear; a raw register dump needs them
                // or a player that writes this byte to $B4 silences the channel.
                bytes
                    .push((u8::from(channel.left) << 7) | (u8::from(channel.right) << 6) | ams_fms);
                bytes.extend(*b"YI");
            }
        }
        bytes
    }

    /// Fields the format lacks take editor defaults: AM off, no LFO
    /// sensitivity, both speakers, global LFO off.
    pub fn decode(self, bytes: &[u8]) -> Result<Instrument, ImportError> {
        if bytes.len() != self.size() {
            return Err(ImportError::Size {
                format: self.name(),
                expected: self.size(),
                actual: bytes.len(),
            });
        }
        let mut instrument = Instrument::default();
        let mut channel = ChannelParameters::default();
        let mut ops = [Operator::default(); 4];
        match self {
            Self::Tfi | Self::Vgi => {
                channel.algorithm = Algorithm::new(bytes[0]).map_err(field("algorithm"))?;
                channel.feedback = value(bytes[1], "feedback")?;
                let mut at = 2;
                if self == Self::Vgi {
                    channel.ams = value(bytes[2] >> 4, "ams")?;
                    channel.fms = value(bytes[2] & 15, "fms")?;
                    at = 3;
                }
                for (op, id) in ops.iter_mut().zip(FILE_ORDER) {
                    let b = &bytes[at..at + 10];
                    let name = |what: &str| format!("{id:?} {what}");
                    let dt = DT_FROM_FILE.get(b[1] as usize).copied().ok_or_else(|| {
                        ImportError::Value {
                            field: name("detune"),
                            source: ParameterError::Range {
                                value: b[1].into(),
                                maximum: 6,
                            },
                        }
                    })?;
                    let (am, d1r) = if self == Self::Vgi {
                        (b[5] & 0x80 != 0, b[5] & 0x7f)
                    } else {
                        (false, b[5])
                    };
                    *op = Operator {
                        mul: value(b[0], &name("mul"))?,
                        dt: Value::new(dt).map_err(field(&name("detune")))?,
                        tl: value(b[2], &name("tl"))?,
                        rs: value(b[3], &name("rs"))?,
                        ar: value(b[4], &name("ar"))?,
                        d1r: value(d1r, &name("d1r"))?,
                        am,
                        d2r: value(b[6], &name("d2r"))?,
                        rr: value(b[7], &name("rr"))?,
                        sl: value(b[8], &name("sl"))?,
                        ssg: SsgEg::try_from(b[9]).map_err(field(&name("ssg")))?,
                    };
                    at += 10;
                }
            }
            Self::Tyi => {
                if &bytes[30..] != b"YI" {
                    return Err(ImportError::Trailer);
                }
                for (i, (op, id)) in ops.iter_mut().zip(FILE_ORDER).enumerate() {
                    let at = |row: usize| bytes[row * 4 + i];
                    let name = |what: &str| format!("{id:?} {what}");
                    *op = Operator {
                        dt: value((at(0) >> 4) & 7, &name("detune"))?,
                        mul: value(at(0) & 15, &name("mul"))?,
                        tl: value(at(1) & 127, &name("tl"))?,
                        rs: value(at(2) >> 6, &name("rs"))?,
                        ar: value(at(2) & 31, &name("ar"))?,
                        am: at(3) & 0x80 != 0,
                        d1r: value(at(3) & 31, &name("d1r"))?,
                        d2r: value(at(4) & 31, &name("d2r"))?,
                        sl: value(at(5) >> 4, &name("sl"))?,
                        rr: value(at(5) & 15, &name("rr"))?,
                        ssg: SsgEg::try_from(at(6)).map_err(field(&name("ssg")))?,
                    };
                }
                channel.algorithm = Algorithm::new(bytes[28] & 7).map_err(field("algorithm"))?;
                channel.feedback = value((bytes[28] >> 3) & 7, "feedback")?;
                channel.ams = value((bytes[29] >> 4) & 3, "ams")?;
                channel.fms = value(bytes[29] & 7, "fms")?;
                // Files written by GENNY never set the pan bits.
                if bytes[29] & 0xc0 != 0 {
                    channel.left = bytes[29] & 0x80 != 0;
                    channel.right = bytes[29] & 0x40 != 0;
                }
            }
        }
        for (op, id) in ops.into_iter().zip(FILE_ORDER) {
            *instrument.operator_mut(id) = op;
        }
        instrument.channel = channel;
        Ok(instrument)
    }

    pub fn read(path: &Path) -> Result<Instrument, ImportError> {
        let format = Self::from_path(path).ok_or(ImportError::Extension)?;
        let bytes = std::fs::read(path)?;
        format.decode(&bytes)
    }
}

impl InstrumentExporter for InstrumentFormat {
    type Error = std::convert::Infallible;
    fn export(&self, instrument: &Instrument) -> Result<Vec<u8>, Self::Error> {
        Ok(self.encode(instrument))
    }
}

fn field(name: &str) -> impl FnOnce(ParameterError) -> ImportError + '_ {
    move |source| ImportError::Value {
        field: name.to_owned(),
        source,
    }
}
fn value<const MAX: u8>(raw: u8, name: &str) -> Result<Value<MAX>, ImportError> {
    Value::new(raw).map_err(field(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn distinct() -> Instrument {
        let mut instrument = Instrument::default();
        for (i, op) in instrument.operators.iter_mut().enumerate() {
            let i = i as u8;
            *op = Operator {
                dt: Value::new([1, 7, 3, 5][i as usize]).unwrap(),
                mul: Value::new(i + 2).unwrap(),
                tl: Value::new(10 + i).unwrap(),
                rs: Value::new(i).unwrap(),
                ar: Value::new(20 + i).unwrap(),
                d1r: Value::new(5 + i).unwrap(),
                d2r: Value::new(1 + i).unwrap(),
                sl: Value::new(3 + i).unwrap(),
                rr: Value::new(7 + i).unwrap(),
                am: i == 2,
                ssg: if i == 1 { SsgEg::ALL[3] } else { SsgEg::OFF },
            };
        }
        instrument.channel.algorithm = Algorithm::new(4).unwrap();
        instrument.channel.feedback = Value::new(6).unwrap();
        instrument.channel.ams = Value::new(2).unwrap();
        instrument.channel.fms = Value::new(5).unwrap();
        instrument
    }

    #[test]
    fn layouts_match_genny_byte_for_byte() {
        let instrument = distinct();
        let tfi = InstrumentFormat::Tfi.encode(&instrument);
        assert_eq!(tfi.len(), 42);
        assert_eq!(&tfi[..2], &[4, 6]);
        // Second file operator is S3: mul 4, detune raw 3 (+3) -> 6, tl 12.
        assert_eq!(&tfi[12..22], &[4, 6, 12, 2, 22, 7, 3, 9, 5, 0]);
        // S2: detune raw 7 (-3) -> 0, SSG 10.
        assert_eq!(&tfi[22..32], &[3, 0, 11, 1, 21, 6, 2, 8, 4, 10]);

        let vgi = InstrumentFormat::Vgi.encode(&instrument);
        assert_eq!(&vgi[..3], &[4, 6, 0x25]);
        // S3 carries AM in bit 7 of D1R.
        assert_eq!(vgi[13 + 5], 0x80 | 7);

        let tyi = InstrumentFormat::Tyi.encode(&instrument);
        assert_eq!(tyi.len(), 32);
        assert_eq!(&tyi[..4], &[0x12, 0x34, 0x73, 0x55]);
        assert_eq!(tyi[28], (6 << 3) | 4);
        assert_eq!(tyi[29], 0xc0 | 0x25);
        assert_eq!(&tyi[30..], b"YI");
    }

    #[test]
    fn every_format_round_trips_what_it_stores() {
        let instrument = distinct();
        for format in InstrumentFormat::ALL {
            let back = format.decode(&format.encode(&instrument)).unwrap();
            let mut expected = instrument.clone();
            if format == InstrumentFormat::Tfi {
                expected.channel.ams = Value::new(0).unwrap();
                expected.channel.fms = Value::new(0).unwrap();
                for op in &mut expected.operators {
                    op.am = false;
                }
            }
            assert_eq!(back, expected, "{format:?}");
            assert_eq!(
                format.omitted(&instrument).is_empty(),
                format != InstrumentFormat::Tfi
            );
        }
    }

    #[test]
    fn negative_zero_detune_and_reports_losses() {
        let mut instrument = Instrument::default();
        instrument.operators[0].dt = Value::new(4).unwrap();
        instrument.channel.left = false;
        instrument.global.lfo_enabled = true;
        let back = InstrumentFormat::Tfi
            .decode(&InstrumentFormat::Tfi.encode(&instrument))
            .unwrap();
        assert_eq!(back.operators[0].dt.get(), 0);
        let omitted = InstrumentFormat::Tfi.omitted(&instrument);
        assert!(omitted.contains(&"panning") && omitted.contains(&"global LFO enable/rate"));
        assert!(
            !InstrumentFormat::Tyi
                .omitted(&instrument)
                .contains(&"panning")
        );
    }

    #[test]
    fn rejects_malformed_files() {
        let mut tfi = InstrumentFormat::Tfi.encode(&Instrument::default());
        assert!(InstrumentFormat::Tfi.decode(&tfi[..41]).is_err());
        tfi[0] = 8;
        assert!(InstrumentFormat::Tfi.decode(&tfi).is_err());
        tfi[0] = 7;
        tfi[2 + 9] = 3;
        assert!(InstrumentFormat::Tfi.decode(&tfi).is_err());
        let mut tyi = InstrumentFormat::Tyi.encode(&Instrument::default());
        tyi[31] = b'X';
        assert!(matches!(
            InstrumentFormat::Tyi.decode(&tyi),
            Err(ImportError::Trailer)
        ));
        // A GENNY TYI without pan bits loads centred.
        let mut genny = InstrumentFormat::Tyi.encode(&Instrument::default());
        genny[29] &= 0x3f;
        let loaded = InstrumentFormat::Tyi.decode(&genny).unwrap();
        assert!(loaded.channel.left && loaded.channel.right);
        assert_eq!(
            InstrumentFormat::from_path(Path::new("a/B.TFI")),
            Some(InstrumentFormat::Tfi)
        );
    }
}
