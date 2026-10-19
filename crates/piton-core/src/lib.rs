//! The deterministic, sans-IO heart of piton.
//!
//! A [`Session`] never touches the outside world. A shell feeds it
//! `(now, Input)` pairs and drains [`Submission`]s; every source of entropy
//! arrives through those inputs, so the same inputs always produce the same
//! submissions.
//!
//! The rules the session upholds:
//!
//! - State is a fold of the log: every change is a record, and
//!   [`Session::from_log`] runs the same fold to recover.
//! - Effects are write-ahead: an effect is dispatched only after storage has
//!   acknowledged the record that declares it.
//! - At most one append is outstanding. Records produced meanwhile go out
//!   together in the next append.
//! - The shell may run dispatched effects concurrently and report their
//!   completions in any order.
//! - Time is an input. Timeouts, grace periods and retry backoff are derived
//!   from the timestamps in the log, so recovery restores them, and the
//!   decisions they lead to are logged like any other.
//! - Outcomes claim only what is known: an effect that may have run is
//!   never reported as not run.

#![no_std]

extern crate alloc;

mod interface;
mod session;
mod state;

pub use interface::{Effect, Input, JournalEntry, Submission};
pub use session::Session;
pub use state::{RecoveryError, SessionState};
