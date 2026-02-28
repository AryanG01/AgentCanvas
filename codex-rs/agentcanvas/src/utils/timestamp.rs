//! Timestamp utilities

use chrono::Utc;

/// Get current Unix timestamp in seconds
pub fn current_timestamp() -> i64 {
    Utc::now().timestamp()
}

/// Parse ISO8601 timestamp to Unix seconds
pub fn parse_iso8601(timestamp: &str) -> Result<i64, chrono::ParseError> {
    timestamp
        .parse::<chrono::DateTime<Utc>>()
        .map(|dt| dt.timestamp())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_current_timestamp() {
        let ts = current_timestamp();
        assert!(ts > 0);
        assert!(ts < 2_000_000_000); // Before year 2033
    }

    #[test]
    fn test_parse_iso8601() {
        let result = parse_iso8601("2024-01-01T00:00:00Z");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 1704067200);
    }
}
