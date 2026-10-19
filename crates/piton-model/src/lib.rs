//! Verification harness for `piton-core`.
//!
//! A [`World`] pairs the real [`piton_core::Session`] with [`Env`], a
//! hand-written model of everything outside it: storage, model providers,
//! tools, clients, crashes and the passage of time. `Env` plays two parts:
//!
//! - [`admits`] is the contract: what any shell may do, whatever the
//!   payload. Real shells are held to it.
//! - [`enabled`] is the generator: the finite part of the contract the
//!   engines explore.
//!
//! [`step`] does one action to the real session and returns the journal
//! entry a shell would write; [`check`] tests the safety invariants, and
//! [`check_exploration`] the ones that only make sense under the generator.
//!
//! Stateright explores every state within small bounds ([`Checked`]); seeded
//! random walks go deep ([`walk`]); journals recorded anywhere replay through
//! [`validate`].

mod check;
mod checker;
mod journal;
mod world;

pub use check::{Violation, check, check_exploration};
pub use checker::Checked;
pub use journal::{Divergence, DivergenceReason, from_ndjson, to_ndjson, validate, walk};
pub use world::{
    Env, EnvAction, World, WorldConfig, admits, enabled, model_policy, recovery, search,
    send_email, step, tools,
};
