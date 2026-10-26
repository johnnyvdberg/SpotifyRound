//! Formatting helpers shared by the UI layer.

/// `m:ss` (or `h:mm:ss` for very long items).
pub fn clock(ms: u64) -> String {
    let total = ms / 1000;
    let (hours, minutes, seconds) = (total / 3600, (total % 3600) / 60, total % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

/// `12 tracks`, `1 track`.
pub fn track_count(count: u32) -> String {
    if count == 1 {
        "1 track".to_owned()
    } else {
        format!("{count} tracks")
    }
}

/// Join the artist names, falling back to the show name for podcasts.
pub fn artists(names: &[String], fallback: &str) -> String {
    if names.is_empty() {
        fallback.to_owned()
    } else {
        names.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_durations() {
        assert_eq!(clock(0), "0:00");
        assert_eq!(clock(9_000), "0:09");
        assert_eq!(clock(61_000), "1:01");
        assert_eq!(clock(3_600_000), "1:00:00");
    }

    #[test]
    fn pluralises_track_counts() {
        assert_eq!(track_count(1), "1 track");
        assert_eq!(track_count(7), "7 tracks");
    }
}
