use std::time::{Duration, SystemTime, UNIX_EPOCH};

const DAYS_BEFORE_MONTH: [u64; 12] = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];

pub fn parse_iso_timestamp(iso: Option<&str>) -> Option<SystemTime> {
    let s = iso?;
    if s.len() < 19 {
        return None;
    }

    let year: u64 = s[0..4].parse().ok()?;
    let month: u64 = s[5..7].parse().ok()?;
    let day: u64 = s[8..10].parse().ok()?;
    let hour: u64 = s[11..13].parse().ok()?;
    let min: u64 = s[14..16].parse().ok()?;
    let sec: u64 = s[17..19].parse().ok()?;

    if !(1970..=3000).contains(&year)
        || !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || min > 59
        || sec > 59
    {
        return None;
    }

    let is_leap = (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400);

    let y = year - 1;
    let leap_years_since_1970 =
        (y / 4 - 1969 / 4) - (y / 100 - 1969 / 100) + (y / 400 - 1969 / 400);
    let mut total_days = (year - 1970) * 365 + leap_years_since_1970;
    total_days += DAYS_BEFORE_MONTH[(month - 1) as usize];
    if month > 2 && is_leap {
        total_days += 1;
    }
    total_days += day - 1;

    let mut total_secs = (total_days * 86400 + hour * 3600 + min * 60 + sec) as i64;

    if s.len() > 19 {
        let remainder = &s[19..];
        if let Some(pos) = remainder.find(['+', '-']) {
            let offset_str = &remainder[pos..];
            if offset_str.len() >= 3 {
                let sign = if &offset_str[0..1] == "+" {
                    1i64
                } else {
                    -1i64
                };
                let off_hour: i64 = offset_str[1..3].parse().unwrap_or(0);
                let off_min: i64 = if offset_str.len() >= 6 && &offset_str[3..4] == ":" {
                    offset_str[4..6].parse().unwrap_or(0)
                } else if offset_str.len() >= 5 {
                    offset_str[3..5].parse().unwrap_or(0)
                } else {
                    0
                };
                let offset_secs = sign * (off_hour * 3600 + off_min * 60);
                total_secs -= offset_secs;
            }
        }
    }

    if total_secs < 0 {
        return None;
    }

    Some(UNIX_EPOCH + Duration::from_secs(total_secs as u64))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_iso_timestamp_none_and_short() {
        assert_eq!(parse_iso_timestamp(None), None);
        assert_eq!(parse_iso_timestamp(Some("2026")), None);
        assert_eq!(parse_iso_timestamp(Some("2026-09-30")), None);
    }

    #[test]
    fn test_parse_iso_timestamp_utc() {
        let ts = parse_iso_timestamp(Some("1970-01-01T00:00:00Z"));
        assert_eq!(ts, Some(UNIX_EPOCH));

        let ts2 = parse_iso_timestamp(Some("1970-01-01T00:01:00Z"));
        assert_eq!(ts2, Some(UNIX_EPOCH + Duration::from_secs(60)));
    }

    #[test]
    fn test_parse_iso_timestamp_with_offset() {
        let ts = parse_iso_timestamp(Some("1970-01-01T01:00:00+01:00"));
        assert_eq!(ts, Some(UNIX_EPOCH));

        let ts2 = parse_iso_timestamp(Some("1970-01-02T05:00:00+05:00"));
        assert_eq!(ts2, Some(UNIX_EPOCH + Duration::from_secs(86400)));
    }

    #[test]
    fn test_parse_iso_timestamp_leap_year() {
        let ts = parse_iso_timestamp(Some("2024-02-29T12:00:00Z"));
        assert!(ts.is_some());
    }

    #[test]
    fn test_parse_iso_timestamp_invalid_ranges() {
        assert_eq!(parse_iso_timestamp(Some("1969-12-31T23:59:59Z")), None);
        assert_eq!(parse_iso_timestamp(Some("2026-13-01T00:00:00Z")), None);
        assert_eq!(parse_iso_timestamp(Some("2026-00-01T00:00:00Z")), None);
        assert_eq!(parse_iso_timestamp(Some("2026-01-32T00:00:00Z")), None);
        assert_eq!(parse_iso_timestamp(Some("2026-01-01T24:00:00Z")), None);
    }
}
