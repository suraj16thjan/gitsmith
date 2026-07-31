use chrono::{DateTime, Utc};

pub fn relative_time(from: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let secs = (now - from).num_seconds().max(0);
    let mins = secs / 60;
    let hours = mins / 60;
    let days = hours / 24;
    if mins < 2 {
        "just now".into()
    } else if hours < 1 {
        format!("{mins}m ago")
    } else if days < 1 {
        format!("{hours}h ago")
    } else {
        format!("{days}d ago")
    }
}

pub fn relative_time_str(iso: &str, now: DateTime<Utc>) -> String {
    match DateTime::parse_from_rfc3339(iso) {
        Ok(dt) => relative_time(dt.with_timezone(&Utc), now),
        Err(_) => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn t(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap()
    }

    #[test]
    fn relative_time_buckets() {
        let now = t(2026, 7, 30, 12, 0);
        assert_eq!(relative_time(t(2026, 7, 30, 11, 59), now), "just now");
        assert_eq!(relative_time(t(2026, 7, 30, 11, 30), now), "30m ago");
        assert_eq!(relative_time(t(2026, 7, 30, 9, 0), now), "3h ago");
        assert_eq!(relative_time(t(2026, 7, 28, 12, 0), now), "2d ago");
    }

    #[test]
    fn relative_time_str_bad_input_is_empty() {
        let now = t(2026, 7, 30, 12, 0);
        assert_eq!(relative_time_str("not-a-date", now), "");
    }
}
