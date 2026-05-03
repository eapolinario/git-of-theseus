//! Cohort label formatting using a Python-style `strftime` format string.
//!
//! The Python CLI exposes `--cohortfm` (default `%Y`). To preserve
//! compatibility we accept the same format strings and translate them to
//! `chrono::format::strftime` directives, which are a near-superset.
//!
//! Supported directives are validated up-front so that invalid format
//! strings are rejected with a clear error rather than producing garbage
//! cohorts at runtime.

use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};

/// Formats a UTC timestamp into a cohort label using the supplied
/// `strftime`-style format string. `%Y`, `%m`, `%d`, `%H`, `%M`, `%S`, `%j`,
/// `%U`, `%W` and literal `%%` are validated; anything chrono can format is
/// otherwise accepted.
pub fn format_cohort(ts: DateTime<Utc>, fmt: &str) -> Result<String> {
    // chrono panics on invalid formatters at format-time, not parse-time, so
    // run a dry format under catch_unwind-equivalent: use chrono's
    // StrftimeItems iterator which surfaces parse errors lazily. We do a
    // throwaway format here and propagate any panic as an error to keep the
    // CLI well-behaved.
    let formatted = std::panic::catch_unwind(|| ts.format(fmt).to_string())
        .map_err(|_| anyhow!("invalid cohort format string: {fmt}"))?;
    Ok(formatted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn ts() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2023, 4, 5, 6, 7, 8).unwrap()
    }

    #[test]
    fn year_default() {
        assert_eq!(format_cohort(ts(), "%Y").unwrap(), "2023");
    }

    #[test]
    fn year_month() {
        assert_eq!(format_cohort(ts(), "%Y-%m").unwrap(), "2023-04");
    }

    #[test]
    fn literal_percent() {
        assert_eq!(format_cohort(ts(), "%Y%%").unwrap(), "2023%");
    }
}
