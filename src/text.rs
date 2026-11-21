// SPDX-License-Identifier: AGPL-3.0-or-later

//! Numbers and dates YouTube only gives as English text ("1.2M views", "3 days ago", "16:48").
//!
//! The connector always queries YouTube in English, so these parsers only know English.

use flaneur_model::{Precision, Published, Seconds, Timestamp};

/// Current time in Unix seconds.
pub(crate) fn now() -> Timestamp {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// The text says how long ago something happened ("3 days ago", "Streamed 2 weeks ago").
pub(crate) fn is_relative(text: &str) -> bool {
    text.split_whitespace()
        .any(|w| w.eq_ignore_ascii_case("ago"))
}

/// Parses "3 days ago", "Streamed 1 hour ago", "Premiered 2 weeks ago", "6 years ago (edited)".
pub(crate) fn parse_ago(text: &str, now: Timestamp) -> Option<Published> {
    let words: Vec<String> = text
        .split_whitespace()
        .map(str::to_ascii_lowercase)
        .collect();
    let ago = words.iter().position(|w| w == "ago")?;
    if ago < 2 {
        return None;
    }
    let n: i64 = match words[ago - 2].as_str() {
        "a" | "an" => 1,
        n => n.parse().ok()?,
    };
    let unit = words[ago - 1].trim_end_matches('s');
    let seconds = match unit {
        "second" => 1,
        "minute" => 60,
        "hour" => 3_600,
        "day" => 86_400,
        "week" => 7 * 86_400,
        "month" => 30 * 86_400,
        "year" => 365 * 86_400,
        _ => return None,
    };
    Some(Published {
        at: now - n * seconds,
        precision: Precision::Relative,
    })
}

/// Parses "2,688,709 views", "1.2M views", "27K views", "1.5K watching", "No views".
pub(crate) fn parse_count(text: &str) -> Option<u64> {
    let first = text.split_whitespace().next()?;
    if first.eq_ignore_ascii_case("no") {
        return Some(0);
    }
    let (number, multiplier) = match first.chars().last()? {
        'K' | 'k' => (&first[..first.len() - 1], 1_000.0),
        'M' => (&first[..first.len() - 1], 1_000_000.0),
        'B' => (&first[..first.len() - 1], 1_000_000_000.0),
        _ => (first, 1.0),
    };
    let number = number.replace(',', "");
    if multiplier > 1.0 {
        let value: f64 = number.parse().ok()?;
        // Counts are far below 2^53: the conversion is exact enough.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        Some((value * multiplier).round() as u64)
    } else {
        number.parse().ok()
    }
}

/// Parses "16:48" or "1:02:03"; `None` for badges that are no duration ("LIVE", "SHORTS").
pub(crate) fn parse_duration(text: &str) -> Option<Seconds> {
    let mut total: Seconds = 0;
    let mut parts = 0;
    for part in text.trim().split(':') {
        let n: Seconds = part.parse().ok()?;
        total = total.checked_mul(60)?.checked_add(n)?;
        parts += 1;
    }
    (2..=3).contains(&parts).then_some(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: Timestamp = 1_800_000_000;

    #[test]
    fn relative_dates_keep_their_precision() {
        let p = parse_ago("3 days ago", NOW).unwrap();
        assert_eq!(p.at, NOW - 3 * 86_400);
        assert_eq!(p.precision, Precision::Relative);
        assert_eq!(
            parse_ago("Streamed 1 hour ago", NOW).unwrap().at,
            NOW - 3_600
        );
        assert_eq!(
            parse_ago("6 years ago (edited)", NOW).unwrap().at,
            NOW - 6 * 365 * 86_400
        );
        assert_eq!(parse_ago("Oct 25, 2009", NOW), None);
        assert!(is_relative("Premiered 2 weeks ago"));
        assert!(!is_relative("Oct 25, 2009"));
    }

    #[test]
    fn counts() {
        assert_eq!(parse_count("2,688,709 views"), Some(2_688_709));
        assert_eq!(parse_count("1.2M views"), Some(1_200_000));
        assert_eq!(parse_count("27K views"), Some(27_000));
        assert_eq!(parse_count("1.5K watching"), Some(1_500));
        assert_eq!(parse_count("No views"), Some(0));
        assert_eq!(parse_count("Linus Tech Tips"), None);
    }

    #[test]
    fn durations() {
        assert_eq!(parse_duration("16:48"), Some(1008));
        assert_eq!(parse_duration("1:02:03"), Some(3723));
        assert_eq!(parse_duration("LIVE"), None);
        assert_eq!(parse_duration("12"), None);
    }
}
