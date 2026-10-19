//! Verification harness for `piton-core`.
//!
//! A [`World`] pairs the real [`piton_core::Session`] with [`Env`], a
//! hand-written model of everything outside it: storage, model providers,
//! tools, clients and crashes. Every engine drives the same three functions:
//!
//! - [`enabled`] lists what the environment may do next,
//! - [`step`] does one of those things to the real session,
//! - [`check`] tests the invariants.
//!
//! Stateright explores every state within small bounds ([`Checked`]); seeded
//! random walks go deep ([`walk`]); traces recorded anywhere replay through
//! [`validate`].

mod check;
mod checker;
mod trace;
mod world;

pub use check::{Violation, check};
pub use checker::Checked;
pub use trace::{Divergence, DivergenceReason, TraceStep, from_ndjson, to_ndjson, validate, walk};
pub use world::{Env, EnvAction, World, WorldConfig, enabled, step};
