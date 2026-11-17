use super::*;

fn instant(text: &str) -> Timestamp {
    parse_rfc3339(text).unwrap()
}

#[test]
fn parses_instants_at_any_offset_and_precision() {
    assert_eq!(
        instant("2026-11-01T09:00:00-03:00"),
        instant("2026-11-01T12:00:00Z")
    );
    assert_eq!(
        instant("2026-11-10T12:00:00.123Z").as_millisecond(),
        instant("2026-11-10T12:00:00Z").as_millisecond() + 123
    );
}

#[test]
fn rejects_text_that_is_not_an_instant() {
    for text in ["", "tomorrow", "2026-11-01", "2026-11-01T09:00:00", "90s"] {
        assert_eq!(parse_rfc3339(text), None, "{text}");
    }
}

#[test]
fn writes_seconds_in_utc_truncating_the_fraction() {
    assert_eq!(
        rfc3339(instant("2026-11-10T09:00:00.999-03:00")),
        "2026-11-10T12:00:00Z"
    );
}

#[test]
fn writes_milliseconds_in_utc_even_when_zero() {
    assert_eq!(
        rfc3339_millis(instant("2026-11-10T12:00:00Z")),
        "2026-11-10T12:00:00.000Z"
    );
    assert_eq!(
        rfc3339_millis(instant("2026-11-10T12:00:00.123999Z")),
        "2026-11-10T12:00:00.123Z"
    );
}

#[test]
fn round_trips_what_it_writes() {
    let written = "2026-11-10T12:00:00Z";
    assert_eq!(rfc3339(instant(written)), written);
    let written = "2026-11-10T12:00:00.250Z";
    assert_eq!(rfc3339_millis(instant(written)), written);
}

#[test]
fn finds_iana_zones_and_their_links_by_exact_name() {
    for name in ["America/Santiago", "UTC", "Etc/UTC", "US/Eastern"] {
        assert_eq!(iana_zone(name).unwrap().iana_name(), Some(name));
    }
}

#[test]
fn rejects_zone_names_that_differ_in_case_or_are_unknown() {
    for name in ["america/santiago", "utc", "Etc/Unknown", "Mars/Olympus", ""] {
        assert!(iana_zone(name).is_none(), "{name}");
    }
}
