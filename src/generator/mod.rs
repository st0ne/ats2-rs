//! Pure generator coding and offline configuration vectors.
//!
//! No instrument commands are implemented here. The exported models calculate
//! software values only and do not perform transport or hardware I/O.

pub mod frequency;
pub mod level;
pub mod sine_vector;

pub use frequency::{
    ActiveRateSource, DspWriteData, ExecutionGate, FrequencyError, FrequencyRuntimeState,
    ModeStatus,
};
pub use level::{
    Channel, EncodedLevel, GeneratorState, ImpedanceState, LevelError, LevelRequest, OutputConfig,
};
pub use sine_vector::{
    ONE_KHZ_HZ, SineModeAssumption, Single1KhzSineBuilder, Single1KhzSineError,
    Single1KhzSineVector,
};
