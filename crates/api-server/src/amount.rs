//! Amounts arrive as decimal strings, matching the JSON the API sends out (u64 can exceed
//! JavaScript's 2^53).

use serde::de::Error;
use serde::{Deserialize, Deserializer};

pub fn required<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
    let text = String::deserialize(deserializer)?;
    return text
        .trim()
        .parse()
        .map_err(|_| D::Error::custom(format!("{text:?} is not a whole number")));
}

pub fn optional<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<u64>, D::Error> {
    let text: Option<String> = Option::deserialize(deserializer)?;
    return match text {
        None => Ok(None),
        Some(text) if text.trim().is_empty() => Ok(None),
        Some(text) => text
            .trim()
            .parse()
            .map(Some)
            .map_err(|_| D::Error::custom(format!("{text:?} is not a whole number"))),
    };
}
