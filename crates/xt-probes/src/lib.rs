//! GitHub probes for explicit pull-request identities, and the shared
//! executable lookup, bounded process runs, group leaders recognised again
//! after a restart, and bounded reads of files left by other programs that
//! probes and judges use.

pub mod executable;
pub mod gh;
pub mod leader;
pub mod process;
pub mod regular_file;

pub use gh::{AttemptClock, CancelFlag, GhClient, GhClientError, Limits, SystemClock};
