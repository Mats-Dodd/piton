use alloc::string::{String, ToString as _};
use core::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer, de, ser};

/// A JSON value the control plane carries but never inspects.
///
/// Tool input and output travel as `Json`. It is held as canonical text
/// (object keys sorted, no insignificant whitespace) so equal values compare
/// and hash equal. Serializes as the JSON value itself, not as a string.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Json(String);

impl Json {
    /// Parses `text` and stores it in canonical form.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidJson`] when `text` is not a JSON document.
    pub fn parse(text: &str) -> Result<Self, InvalidJson> {
        let invalid = |error: serde_json::Error| InvalidJson {
            reason: error.to_string(),
        };
        let value: serde_json::Value = serde_json::from_str(text).map_err(invalid)?;
        serde_json::to_string(&value).map(Self).map_err(invalid)
    }

    /// A JSON string holding `text`.
    ///
    /// # Panics
    ///
    /// Never in practice: serializing a `&str` cannot fail.
    #[must_use]
    pub fn string(text: &str) -> Self {
        Self(serde_json::to_string(text).expect("a string serializes"))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Serialize for Json {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let value: serde_json::Value = serde_json::from_str(&self.0).map_err(ser::Error::custom)?;
        value.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Json {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        serde_json::to_string(&value)
            .map(Self)
            .map_err(de::Error::custom)
    }
}

/// Text that failed to parse as JSON.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvalidJson {
    reason: String,
}

impl fmt::Display for InvalidJson {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid JSON: {}", self.reason)
    }
}

impl core::error::Error for InvalidJson {}
