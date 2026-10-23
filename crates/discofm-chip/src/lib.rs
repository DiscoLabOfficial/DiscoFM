//! Validated hardware values and YM2612 register encoding. Operators use S1..S4
//! names from the datasheet, with register offsets 0, 8, 4, 12 respectively.
#![forbid(unsafe_code)]

mod algorithm;
pub use algorithm::{Algorithm, AlgorithmRouting, OperatorId};

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ParameterError {
    #[error("register value {value} exceeds maximum {maximum}")]
    Range { value: u16, maximum: u16 },
    #[error("SSG-EG must be off (0) or an enabled encoding 8..15, got {0}")]
    Ssg(u8),
    #[error("channel 3 special frequencies require channel 3")]
    SpecialChannel,
    #[error("hardware channel must be 1..6, got {0}")]
    Channel(u8),
}

/// Inclusive unsigned hardware range; deserialization validates the value too.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
pub struct Value<const MAX: u8>(u8);

impl<const MAX: u8> Value<MAX> {
    pub const fn new(value: u8) -> Result<Self, ParameterError> {
        if value <= MAX {
            Ok(Self(value))
        } else {
            Err(ParameterError::Range {
                value: value as u16,
                maximum: MAX as u16,
            })
        }
    }
    pub const fn get(self) -> u8 {
        self.0
    }
}
impl<const MAX: u8> Default for Value<MAX> {
    fn default() -> Self {
        Self(0)
    }
}
impl<const MAX: u8> TryFrom<u8> for Value<MAX> {
    type Error = ParameterError;
    fn try_from(value: u8) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}
impl<const MAX: u8> From<Value<MAX>> for u8 {
    fn from(value: Value<MAX>) -> Self {
        value.0
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
pub struct SsgEg(u8);
impl SsgEg {
    pub const OFF: Self = Self(0);
    pub const ALL: [Self; 9] = [
        Self(0),
        Self(8),
        Self(9),
        Self(10),
        Self(11),
        Self(12),
        Self(13),
        Self(14),
        Self(15),
    ];
    pub const fn from_shape(shape: Value<7>) -> Self {
        Self(8 | shape.get())
    }
    pub const fn from_flags(inverted: bool, alternate: bool, hold: bool) -> Self {
        Self(8 | ((inverted as u8) << 2) | ((alternate as u8) << 1) | hold as u8)
    }
    pub const fn is_enabled(self) -> bool {
        self.0 & 8 != 0
    }
    pub const fn inverted(self) -> bool {
        self.0 & 4 != 0
    }
    pub const fn alternate(self) -> bool {
        self.0 & 2 != 0
    }
    pub const fn hold(self) -> bool {
        self.0 & 1 != 0
    }
    pub const fn shape(self) -> Option<Value<7>> {
        if self.is_enabled() {
            Some(Value(self.0 & 7))
        } else {
            None
        }
    }
    pub const fn raw(self) -> u8 {
        self.0
    }
}
impl TryFrom<u8> for SsgEg {
    type Error = ParameterError;
    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 | 8..=15 => Ok(Self(value)),
            _ => Err(ParameterError::Ssg(value)),
        }
    }
}
impl From<SsgEg> for u8 {
    fn from(value: SsgEg) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Operator {
    pub dt: Value<7>,
    pub mul: Value<15>,
    pub tl: Value<127>,
    pub rs: Value<3>,
    pub ar: Value<31>,
    pub d1r: Value<31>,
    pub d2r: Value<31>,
    pub sl: Value<15>,
    pub rr: Value<15>,
    pub am: bool,
    pub ssg: SsgEg,
}
impl Operator {
    /// Musician-facing magnitude/sign; raw DT=4 (negative zero) stays preserved.
    pub const fn signed_detune(self) -> i8 {
        [0, 1, 2, 3, 0, -1, -2, -3][self.dt.get() as usize]
    }
    /// Display ratio only. Actual oscillator math remains inside the chip core.
    pub const fn multiplier_ratio(self) -> (u8, u8) {
        if self.mul.get() == 0 {
            (1, 2)
        } else {
            (self.mul.get(), 1)
        }
    }
}
impl Default for Operator {
    fn default() -> Self {
        Self {
            dt: Value(0),
            mul: Value(1),
            tl: Value(127),
            rs: Value(0),
            ar: Value(31),
            d1r: Value(0),
            d2r: Value(0),
            sl: Value(0),
            rr: Value(10),
            am: false,
            ssg: SsgEg::OFF,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelParameters {
    pub algorithm: Algorithm,
    pub feedback: Value<7>,
    pub ams: Value<3>,
    pub fms: Value<7>,
    pub left: bool,
    pub right: bool,
}
impl Default for ChannelParameters {
    fn default() -> Self {
        Self {
            algorithm: Algorithm::PARALLEL,
            feedback: Value(0),
            ams: Value(0),
            fms: Value(0),
            left: true,
            right: true,
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GlobalParameters {
    pub lfo_enabled: bool,
    pub lfo_rate: Value<7>,
}

/// Software-only note-on attenuation depths, separate from hardware parameters.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditionSettings {
    pub velocity_tl_depth: [Value<127>; 4],
}
impl AuditionSettings {
    pub fn total_level(
        &self,
        operator: OperatorId,
        level: Value<127>,
        velocity: Value<127>,
    ) -> Value<127> {
        let extra = (u16::from(127 - velocity.get())
            * u16::from(self.velocity_tl_depth[operator.index()].get())
            + 63)
            / 127;
        Value((u16::from(level.get()) + extra).min(127) as u8)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "(u16, u8)", into = "(u16, u8)")]
pub struct Frequency {
    fnum: u16,
    block: u8,
}
impl Frequency {
    pub fn new(fnum: u16, block: u8) -> Result<Self, ParameterError> {
        if fnum > 2047 {
            return Err(ParameterError::Range {
                value: fnum,
                maximum: 2047,
            });
        }
        if block > 7 {
            return Err(ParameterError::Range {
                value: block.into(),
                maximum: 7,
            });
        }
        Ok(Self { fnum, block })
    }
    pub const fn fnum(self) -> u16 {
        self.fnum
    }
    pub const fn block(self) -> u8 {
        self.block
    }
    pub const fn high(self) -> u8 {
        (self.block << 3) | ((self.fnum >> 8) as u8)
    }
    pub const fn low(self) -> u8 {
        self.fnum as u8
    }
}
impl TryFrom<(u16, u8)> for Frequency {
    type Error = ParameterError;
    fn try_from(value: (u16, u8)) -> Result<Self, Self::Error> {
        Self::new(value.0, value.1)
    }
}
impl From<Frequency> for (u16, u8) {
    fn from(value: Frequency) -> Self {
        (value.fnum, value.block)
    }
}

/// Hardware channel numbered 1..6; bank and key-on codes are derived, not guessed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
pub struct Channel(u8);
impl Channel {
    pub const ONE: Self = Self(1);
    pub const THREE: Self = Self(3);
    pub fn new(number: u8) -> Result<Self, ParameterError> {
        if (1..=6).contains(&number) {
            Ok(Self(number))
        } else {
            Err(ParameterError::Channel(number))
        }
    }
    pub const fn number(self) -> u8 {
        self.0
    }
    pub const fn bank(self) -> u8 {
        (self.0 - 1) / 3
    }
    pub const fn offset(self) -> u8 {
        (self.0 - 1) % 3
    }
    pub const fn key_code(self) -> u8 {
        self.offset() | (self.bank() << 2)
    }
}
impl TryFrom<u8> for Channel {
    type Error = ParameterError;
    fn try_from(value: u8) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}
impl From<Channel> for u8 {
    fn from(value: Channel) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Instrument {
    pub operators: [Operator; 4],
    pub channel: ChannelParameters,
    pub global: GlobalParameters,
    /// S1, S2, S3 absolute hardware frequencies; S4 uses the audition pitch.
    pub channel3_special: Option<[Frequency; 3]>,
    pub audition: AuditionSettings,
}
impl Instrument {
    pub fn required_channel(&self) -> Option<Channel> {
        self.channel3_special.map(|_| Channel::THREE)
    }
    pub fn validate_channel(&self, channel: Channel) -> Result<(), ParameterError> {
        if let Some(required) = self.required_channel()
            && channel != required
        {
            return Err(ParameterError::SpecialChannel);
        }
        Ok(())
    }
    pub fn operator(&self, operator: OperatorId) -> &Operator {
        &self.operators[operator.index()]
    }
    pub fn operator_mut(&mut self, operator: OperatorId) -> &mut Operator {
        &mut self.operators[operator.index()]
    }
    /// S1..S4 frequencies for audition. Special-mode S1..S3 are absolute values;
    /// S4 continues to use the channel frequency passed here.
    pub fn operator_frequencies(&self, pitch: Frequency) -> [Frequency; 4] {
        match self.channel3_special {
            Some([s1, s2, s3]) => [s1, s2, s3, pitch],
            None => [pitch; 4],
        }
    }
}
impl Default for Instrument {
    fn default() -> Self {
        let mut operators = [Operator::default(); 4];
        operators[3].tl = Value(24);
        Self {
            operators,
            channel: ChannelParameters::default(),
            global: GlobalParameters::default(),
            channel3_special: None,
            audition: AuditionSettings::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisterWrite {
    pub bank: u8,
    pub address: u8,
    pub data: u8,
}
impl RegisterWrite {
    pub const fn new(bank: u8, address: u8, data: u8) -> Self {
        Self {
            bank,
            address,
            data,
        }
    }
}

/// Setup writes only. Engine owns bus timing, frequency and note events.
pub fn encode_instrument(
    patch: &Instrument,
    channel: Channel,
    velocity: Value<127>,
) -> Result<Vec<RegisterWrite>, ParameterError> {
    patch.validate_channel(channel)?;
    let mut writes = vec![
        RegisterWrite::new(
            0,
            0x22,
            (u8::from(patch.global.lfo_enabled) << 3) | patch.global.lfo_rate.get(),
        ),
        RegisterWrite::new(
            0,
            0x27,
            if patch.channel3_special.is_some() {
                0x40
            } else {
                0
            },
        ),
        RegisterWrite::new(0, 0x2b, 0),
    ];
    for (i, op) in patch.operators.iter().enumerate() {
        let operator = OperatorId::ALL[i];
        let offset = operator.register_offset() + channel.offset();
        let tl = patch.audition.total_level(operator, op.tl, velocity).get();
        for (base, data) in [
            (0x30, (op.dt.get() << 4) | op.mul.get()),
            (0x40, tl),
            (0x50, (op.rs.get() << 6) | op.ar.get()),
            (0x60, (u8::from(op.am) << 7) | op.d1r.get()),
            (0x70, op.d2r.get()),
            (0x80, (op.sl.get() << 4) | op.rr.get()),
            (0x90, op.ssg.raw()),
        ] {
            writes.push(RegisterWrite::new(channel.bank(), base + offset, data));
        }
    }
    let ch = patch.channel;
    writes.push(RegisterWrite::new(
        channel.bank(),
        0xb0 + channel.offset(),
        (ch.feedback.get() << 3) | ch.algorithm.get(),
    ));
    writes.push(RegisterWrite::new(
        channel.bank(),
        0xb4 + channel.offset(),
        (u8::from(ch.left) << 7) | (u8::from(ch.right) << 6) | (ch.ams.get() << 4) | ch.fms.get(),
    ));
    if let Some(frequencies) = patch.channel3_special {
        for (f, low) in frequencies.into_iter().zip([0xa9, 0xaa, 0xa8]) {
            writes.push(RegisterWrite::new(0, low + 4, f.high()));
            writes.push(RegisterWrite::new(0, low, f.low()));
        }
    }
    Ok(writes)
}

pub fn frequency_writes(channel: Channel, frequency: Frequency) -> [RegisterWrite; 2] {
    [
        RegisterWrite::new(channel.bank(), 0xa4 + channel.offset(), frequency.high()),
        RegisterWrite::new(channel.bank(), 0xa0 + channel.offset(), frequency.low()),
    ]
}
pub fn key_write(channel: Channel, operators: Value<15>) -> RegisterWrite {
    RegisterWrite::new(0, 0x28, (operators.get() << 4) | channel.key_code())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_invalid_values_including_json() {
        assert!(Value::<7>::new(8).is_err());
        assert!(serde_json::from_str::<Value<31>>("32").is_err());
        assert!(Frequency::new(2048, 0).is_err());
        assert!(Frequency::new(1, 8).is_err());
        assert!(Channel::new(0).is_err());
        for raw in 0..=255 {
            assert_eq!(
                SsgEg::try_from(raw).is_ok(),
                raw == 0 || (8..=15).contains(&raw)
            );
        }
    }
    #[test]
    fn all_six_channels_and_four_operator_slots() {
        for number in 1..=6 {
            let channel = Channel::new(number).unwrap();
            let mut patch = Instrument::default();
            for (i, op) in patch.operators.iter_mut().enumerate() {
                op.tl = Value::new(i as u8).unwrap();
            }
            let writes = encode_instrument(&patch, channel, Value(127)).unwrap();
            for (i, slot) in [0, 8, 4, 12].into_iter().enumerate() {
                assert!(writes.contains(&RegisterWrite::new(
                    channel.bank(),
                    0x40 + slot + channel.offset(),
                    i as u8
                )));
            }
            assert_eq!(
                key_write(channel, Value(15)).data,
                0xf0 | [0, 1, 2, 4, 5, 6][number as usize - 1]
            );
        }
    }
    #[test]
    fn bitfields_preserve_hardware_encodings() {
        let mut patch = Instrument::default();
        patch.operators[0] = Operator {
            dt: Value(7),
            mul: Value(0),
            tl: Value(127),
            rs: Value(3),
            ar: Value(31),
            d1r: Value(31),
            d2r: Value(31),
            sl: Value(15),
            rr: Value(15),
            am: true,
            ssg: SsgEg(15),
        };
        patch.channel = ChannelParameters {
            algorithm: Algorithm::PARALLEL,
            feedback: Value(7),
            ams: Value(3),
            fms: Value(7),
            left: true,
            right: false,
        };
        patch.global = GlobalParameters {
            lfo_enabled: true,
            lfo_rate: Value(7),
        };
        let writes = encode_instrument(&patch, Channel::ONE, Value(127)).unwrap();
        for (address, data) in [
            (0x22, 15),
            (0x30, 0x70),
            (0x40, 127),
            (0x50, 0xdf),
            (0x60, 0x9f),
            (0x70, 31),
            (0x80, 0xff),
            (0x90, 15),
            (0xb0, 0x3f),
            (0xb4, 0xb7),
        ] {
            assert!(writes.contains(&RegisterWrite::new(0, address, data)));
        }
    }
    #[test]
    fn special_mode_frequency_latches_and_velocity() {
        let mut patch = Instrument {
            channel3_special: Some([
                Frequency::new(700, 3).unwrap(),
                Frequency::new(1100, 4).unwrap(),
                Frequency::new(1500, 2).unwrap(),
            ]),
            ..Instrument::default()
        };
        patch.audition.velocity_tl_depth[3] = Value(127);
        assert!(encode_instrument(&patch, Channel::ONE, Value(127)).is_err());
        let writes = encode_instrument(&patch, Channel::THREE, Value(0)).unwrap();
        assert!(writes.contains(&RegisterWrite::new(0, 0x4e, 127)));
        for (low, frequency) in [0xa9, 0xaa, 0xa8]
            .into_iter()
            .zip(patch.channel3_special.unwrap())
        {
            let pos = writes.iter().position(|w| w.address == low).unwrap();
            assert_eq!(writes[pos - 1].address, low + 4);
            assert_eq!(writes[pos - 1].data, frequency.high());
            assert_eq!(writes[pos].data, frequency.low());
        }
    }
}
