use alloc::string::String;

use serde::{Deserialize, Serialize};

/// Position of a record in a session's log. The first record sits at offset 0,
/// so an offset also counts the records before it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Offset(u64);

impl Offset {
    pub const ZERO: Self = Self(0);

    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    /// The offset directly after this one.
    ///
    /// # Panics
    ///
    /// Panics if the log would grow past `u64::MAX` records.
    #[must_use]
    pub const fn next(self) -> Self {
        Self(self.0.checked_add(1).expect("log offset overflowed u64"))
    }
}

/// Identifies an effect: the offset of the record that declared its intent.
/// Stable across restarts, so it doubles as the effect's idempotency key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct EffectId(Offset);

impl EffectId {
    #[must_use]
    pub const fn new(intent: Offset) -> Self {
        Self(intent)
    }

    #[must_use]
    pub const fn intent(self) -> Offset {
        self.0
    }
}

/// Identifies a turn: the offset of its `TurnStarted` record.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TurnId(Offset);

impl TurnId {
    #[must_use]
    pub const fn new(start: Offset) -> Self {
        Self(start)
    }

    #[must_use]
    pub const fn start(self) -> Offset {
        self.0
    }
}

/// Wall-clock milliseconds since the Unix epoch, as reported by the shell.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Timestamp(u64);

impl Timestamp {
    pub const EPOCH: Self = Self(0);

    #[must_use]
    pub const fn from_millis(millis: u64) -> Self {
        Self(millis)
    }

    #[must_use]
    pub const fn millis(self) -> u64 {
        self.0
    }

    /// The instant `duration` after this one, saturating at the far future.
    #[must_use]
    pub const fn after(self, duration: Millis) -> Self {
        Self(self.0.saturating_add(duration.0))
    }

    /// Time from `earlier` to this instant, or zero if `earlier` is later.
    #[must_use]
    pub const fn since(self, earlier: Self) -> Millis {
        Millis(self.0.saturating_sub(earlier.0))
    }
}

/// A span of time in milliseconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Millis(u64);

impl Millis {
    pub const ZERO: Self = Self(0);

    #[must_use]
    pub const fn new(millis: u64) -> Self {
        Self(millis)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

macro_rules! string_id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        pub struct $name(String);

        impl $name {
            #[must_use]
            pub const fn new(value: String) -> Self {
                Self(value)
            }

            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
    };
}

string_id! {
    /// Names a session; each session has its own log.
    SessionId
}

string_id! {
    /// Chosen by a client so a retried submission is admitted at most once.
    RequestId
}

string_id! {
    /// Chosen by the model to pair a tool use with its result.
    ToolUseId
}

string_id! {
    /// Names a tool the session may call.
    ToolName
}
