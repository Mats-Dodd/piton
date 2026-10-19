//! Runs piton sessions on tokio.
//!
//! This is the embeddable host: give a [`Runtime`] its [`Ports`] (storage,
//! a model provider, a tool host, a clock and a journal) and it runs each
//! session as one task that executes the core's work. Servers, terminal
//! apps and embedders all build on it.
//!
//! The runtime makes no decisions; the core makes them all. When an append
//! fails the runtime cannot know whether it landed, so it does what a crash
//! would do: stops every effect, ignores anything they still report, and
//! rebuilds the session from the log.

mod basic;
mod ports;
mod pump;
mod runtime;

pub use basic::{MemLogStore, NoopTelemetryJournal, SystemClock, TokioClock};
pub use ports::{
    CancelSignal, Clock, LogStore, ModelError, ModelProvider, StoreError, TelemetryJournal,
    ToolCompletion, ToolHost,
};
pub use pump::OpenError;
pub use runtime::{FollowError, Follower, Ports, Runtime, SessionClosed, SessionHandle};
