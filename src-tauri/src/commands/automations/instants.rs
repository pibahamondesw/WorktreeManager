//! Instants and time zones as automations read and write them.

use jiff::tz::TimeZone;
use jiff::Timestamp;

/// An RFC 3339 instant at any offset, as definitions, stores and forges write it.
pub fn parse_rfc3339(text: &str) -> Option<Timestamp> {
    text.parse().ok()
}

/// RFC 3339 in UTC to the second, the fraction truncated.
pub fn rfc3339(instant: Timestamp) -> String {
    format!("{instant:.0}")
}

/// RFC 3339 in UTC to the millisecond, the rest truncated.
pub fn rfc3339_millis(instant: Timestamp) -> String {
    format!("{instant:.3}")
}

/// The IANA time zone spelled exactly as `name`.
pub fn iana_zone(name: &str) -> Option<TimeZone> {
    let zone = TimeZone::get(name).ok()?;
    let spelled_exactly = zone.iana_name() == Some(name) && !zone.is_unknown();
    spelled_exactly.then_some(zone)
}

#[cfg(test)]
mod tests;
