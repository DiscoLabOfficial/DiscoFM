//! Datasheet operator identities and immutable routing for all hardware algorithms.
use crate::{ParameterError, Value};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OperatorId {
    S1,
    S2,
    S3,
    S4,
}
impl OperatorId {
    pub const ALL: [Self; 4] = [Self::S1, Self::S2, Self::S3, Self::S4];
    pub const fn index(self) -> usize {
        self as usize
    }
    pub const fn number(self) -> u8 {
        self as u8 + 1
    }
    pub const fn register_offset(self) -> u8 {
        [0, 8, 4, 12][self.index()]
    }
}

/// Numeric serialization remains the raw 0..7 register value in version 1 sessions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
pub struct Algorithm(Value<7>);
impl Algorithm {
    pub const ALL: [Self; 8] = [
        Self(Value(0)),
        Self(Value(1)),
        Self(Value(2)),
        Self(Value(3)),
        Self(Value(4)),
        Self(Value(5)),
        Self(Value(6)),
        Self(Value(7)),
    ];
    pub const PARALLEL: Self = Self(Value(7));
    pub const fn new(raw: u8) -> Result<Self, ParameterError> {
        match Value::new(raw) {
            Ok(value) => Ok(Self(value)),
            Err(error) => Err(error),
        }
    }
    pub const fn get(self) -> u8 {
        self.0.get()
    }
    pub const fn display_number(self) -> u8 {
        self.get() + 1
    }
    pub const fn feedback_operator(self) -> OperatorId {
        OperatorId::S1
    }
    pub const fn routing(self) -> AlgorithmRouting {
        use OperatorId::*;
        match self.get() {
            0 => AlgorithmRouting {
                connections: &[(S1, S2), (S2, S3), (S3, S4)],
                carriers: &[S4],
            },
            1 => AlgorithmRouting {
                connections: &[(S1, S3), (S2, S3), (S3, S4)],
                carriers: &[S4],
            },
            2 => AlgorithmRouting {
                connections: &[(S1, S4), (S2, S3), (S3, S4)],
                carriers: &[S4],
            },
            3 => AlgorithmRouting {
                connections: &[(S1, S2), (S2, S4), (S3, S4)],
                carriers: &[S4],
            },
            4 => AlgorithmRouting {
                connections: &[(S1, S2), (S3, S4)],
                carriers: &[S2, S4],
            },
            5 => AlgorithmRouting {
                connections: &[(S1, S2), (S1, S3), (S1, S4)],
                carriers: &[S2, S3, S4],
            },
            6 => AlgorithmRouting {
                connections: &[(S1, S2)],
                carriers: &[S2, S3, S4],
            },
            // The validated constructor makes 7 the only remaining value.
            _ => AlgorithmRouting {
                connections: &[],
                carriers: &[S1, S2, S3, S4],
            },
        }
    }
    pub fn is_carrier(self, operator: OperatorId) -> bool {
        self.routing().carriers.contains(&operator)
    }
}
impl TryFrom<u8> for Algorithm {
    type Error = ParameterError;
    fn try_from(raw: u8) -> Result<Self, Self::Error> {
        Self::new(raw)
    }
}
impl From<Algorithm> for u8 {
    fn from(value: Algorithm) -> Self {
        value.get()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct AlgorithmRouting {
    /// Modulation edges only; S1's feedback loop is represented separately.
    pub connections: &'static [(OperatorId, OperatorId)],
    pub carriers: &'static [OperatorId],
}
