//! In-process runners: deterministic fakes, a hand-driven runner for state
//! and abort testing, and the model runner (a model device is just a
//! ToolService with a [`model::ModelRunner`] behind it).
//!
//! The process runner — real executables behind host policy — is explicitly
//! outside this crate (`docs/toolfs.md` §"Runner Boundary").

pub mod fake;
pub mod manual;
pub mod model;

pub use fake::{EchoRunner, FailRunner, UpperRunner};
pub use manual::{ManualHandle, ManualRunner};
pub use model::{FakeModelEngine, ModelEngine, ModelRunner};
