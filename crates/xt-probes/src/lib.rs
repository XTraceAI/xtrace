//! GitHub probes for explicit pull-request identities.

pub mod gh;

pub use gh::{AttemptClock, CancelFlag, GhClient, GhClientError, Limits, SystemClock};
