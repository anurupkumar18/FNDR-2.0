//! Lifecycle-owned session identity for capture (T-307).
//!
//! The identity *policy* lives in `fndr_memory::continuity`, ported from v1
//! under ADR-005. That crate deliberately owns no timezone: it takes an
//! already-resolved local day and minute-of-day. This module is the
//! application boundary that resolves them, so the shell (the capture
//! lifecycle owner) supplies wall-clock context and the engine keeps the one
//! authoritative interpretation of the 30-minute scheme. Nothing here
//! re-derives, re-formats, or second-guesses that policy.

use std::sync::Once;

use fndr_memory::{SessionIdentityError, build_session_id, build_session_key};

/// A local civil time, as the ported policy needs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalCivilTime {
    /// `YYYYMMDD` in the machine's local timezone.
    pub day_yyyymmdd: [u8; 8],
    /// Minutes since local midnight, `0..1440`.
    pub minute_of_day: u16,
}

impl LocalCivilTime {
    pub fn day(&self) -> &str {
        // Constructed only from ASCII digits by the constructors below.
        std::str::from_utf8(&self.day_yyyymmdd).expect("local day is ASCII digits")
    }
}

/// Wall-clock to local civil time. It is a trait so the capture tests can pin
/// a timezone instead of depending on the machine running them.
pub trait LocalClock: Send + Sync + 'static {
    fn local_civil_time(&self, unix_ms: i64) -> Option<LocalCivilTime>;
}

/// Failure to derive session identity is a typed, visible state (invariant 4).
/// There is deliberately no "unknown session" fallback id: a record whose
/// session cannot be derived must not be persisted under an invented identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SessionIdentityUnavailable {
    #[error("local civil time is unavailable for the capture timestamp")]
    LocalTimeUnavailable,
    #[error("continuity policy rejected the resolved local day")]
    InvalidDay,
    #[error("continuity policy rejected the resolved minute of day")]
    InvalidMinute,
}

impl From<SessionIdentityError> for SessionIdentityUnavailable {
    fn from(error: SessionIdentityError) -> Self {
        match error {
            SessionIdentityError::InvalidDay => Self::InvalidDay,
            SessionIdentityError::InvalidMinute => Self::InvalidMinute,
        }
    }
}

/// The derived identity of one capture. `session_key` is retained because it
/// is the policy's own context key; callers must never rebuild it themselves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureSessionIdentity {
    pub session_id: String,
    pub session_key: String,
}

/// The context a capture presents to the identity policy.
#[derive(Debug, Clone, Copy)]
pub struct SessionContext<'a> {
    pub app_name: &'a str,
    pub bundle_id: Option<&'a str>,
    pub window_title: &'a str,
    pub url: Option<&'a str>,
    pub captured_at_ms: i64,
}

/// Derives capture session identity from the ported continuity policy.
pub struct SessionIdentityDeriver {
    clock: Box<dyn LocalClock>,
}

impl SessionIdentityDeriver {
    pub fn new(clock: impl LocalClock) -> Self {
        Self {
            clock: Box::new(clock),
        }
    }

    /// The default composition for the desktop lifecycle owner.
    pub fn system() -> Self {
        Self::new(SystemLocalClock)
    }

    pub fn derive(
        &self,
        context: SessionContext<'_>,
    ) -> Result<CaptureSessionIdentity, SessionIdentityUnavailable> {
        let civil = self
            .clock
            .local_civil_time(context.captured_at_ms)
            .ok_or(SessionIdentityUnavailable::LocalTimeUnavailable)?;
        let session_key = build_session_key(context.app_name, context.window_title, context.url);
        let session_id = build_session_id(
            civil.day(),
            civil.minute_of_day,
            context.app_name,
            context.bundle_id,
            &session_key,
        )?;
        Ok(CaptureSessionIdentity {
            session_id,
            session_key,
        })
    }
}

impl Default for SessionIdentityDeriver {
    fn default() -> Self {
        Self::system()
    }
}

/// The machine's own timezone database, via `localtime_r`.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemLocalClock;

impl LocalClock for SystemLocalClock {
    #[cfg(unix)]
    fn local_civil_time(&self, unix_ms: i64) -> Option<LocalCivilTime> {
        // POSIX does not require `localtime_r` to consult TZ itself (glibc
        // deliberately does not), so the zone is resolved once, explicitly,
        // and every capture thread then shares it.
        static TZSET: Once = Once::new();
        // `tzset` is POSIX but is not re-exported by the `libc` crate on
        // every target, so it is declared here rather than shimmed around.
        unsafe extern "C" {
            fn tzset();
        }
        TZSET.call_once(|| unsafe { tzset() });

        let seconds: libc::time_t = unix_ms.div_euclid(1_000).try_into().ok()?;
        let mut resolved: libc::tm = unsafe { std::mem::zeroed() };
        let returned = unsafe { libc::localtime_r(&seconds, &mut resolved) };
        if returned.is_null() {
            return None;
        }
        civil_from_tm(&resolved)
    }

    #[cfg(not(unix))]
    fn local_civil_time(&self, _unix_ms: i64) -> Option<LocalCivilTime> {
        // No silent degradation: an unsupported host reports unavailability
        // rather than pretending the capture happened in UTC.
        None
    }
}

#[cfg(unix)]
fn civil_from_tm(resolved: &libc::tm) -> Option<LocalCivilTime> {
    let year = i64::from(resolved.tm_year).checked_add(1_900)?;
    let month = i64::from(resolved.tm_mon).checked_add(1)?;
    let day = i64::from(resolved.tm_mday);
    if !(1..=9_999).contains(&year) || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let hour = u16::try_from(resolved.tm_hour).ok()?;
    let minute = u16::try_from(resolved.tm_min).ok()?;
    if hour >= 24 || minute >= 60 {
        return None;
    }
    Some(LocalCivilTime {
        day_yyyymmdd: format!("{year:04}{month:02}{day:02}")
            .into_bytes()
            .try_into()
            .ok()?,
        minute_of_day: hour * 60 + minute,
    })
}

/// A fixed UTC offset, so identity tests never depend on the machine's
/// timezone. The civil arithmetic here is test scaffolding only; production
/// resolves the platform database above.
#[cfg(test)]
pub(crate) struct FixedOffsetClock {
    pub(crate) offset_minutes: i64,
}

#[cfg(test)]
impl LocalClock for FixedOffsetClock {
    fn local_civil_time(&self, unix_ms: i64) -> Option<LocalCivilTime> {
        let local_minutes = unix_ms
            .div_euclid(60_000)
            .checked_add(self.offset_minutes)?;
        let days = local_minutes.div_euclid(1_440);
        let minute_of_day = u16::try_from(local_minutes.rem_euclid(1_440)).ok()?;
        let (year, month, day) = civil_from_days(days);
        Some(LocalCivilTime {
            day_yyyymmdd: format!("{year:04}{month:02}{day:02}")
                .into_bytes()
                .try_into()
                .ok()?,
            minute_of_day,
        })
    }
}

// Howard Hinnant's civil-from-days; test-only.
#[cfg(test)]
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    };
    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deriver() -> SessionIdentityDeriver {
        SessionIdentityDeriver::new(FixedOffsetClock {
            offset_minutes: -7 * 60,
        })
    }

    fn context<'a>(
        title: &'a str,
        url: Option<&'a str>,
        captured_at_ms: i64,
    ) -> SessionContext<'a> {
        SessionContext {
            app_name: "Google Chrome",
            bundle_id: Some("com.google.Chrome"),
            window_title: title,
            url,
            captured_at_ms,
        }
    }

    // 2026-05-07T04:46:00Z, which is 2026-05-06 21:46 at the -07:00 offset
    // above: the same civil instant the ported policy's own test pins, so a
    // drift between the two layers surfaces as a value mismatch here.
    const BASE_MS: i64 = 1_778_129_160_000;

    #[test]
    fn identity_matches_the_ported_policy_for_a_known_instant() {
        let identity = deriver()
            .derive(context(
                "Screenpipe Architecture Deep Dive",
                Some("https://docs.screenpi.pe/architecture/memory-cards?secret=no"),
                BASE_MS,
            ))
            .unwrap();

        assert_eq!(
            identity.session_key,
            "google_chrome:docs_screenpi_pe:screenpipe_architecture_deep_dive"
        );
        assert_eq!(
            identity.session_id,
            "20260506-com.google.chrome-docs_screenpi_pe-s43"
        );
    }

    #[test]
    fn identity_is_stable_inside_the_window_and_rolls_over_at_the_boundary() {
        let deriver = deriver();
        let url = Some("https://docs.screenpi.pe/architecture/memory-cards");
        let first = deriver
            .derive(context("Deep Dive", url, BASE_MS))
            .unwrap()
            .session_id;
        let later = deriver
            .derive(context("Deep Dive", url, BASE_MS + 3 * 60_000))
            .unwrap()
            .session_id;
        // 21:46 local sits in bucket s43 (21:30..22:00); +14 minutes crosses
        // the policy boundary into s44 at 22:00.
        let after_boundary = deriver
            .derive(context("Deep Dive", url, BASE_MS + 14 * 60_000))
            .unwrap()
            .session_id;

        assert_eq!(first, later);
        assert_eq!(first, "20260506-com.google.chrome-docs_screenpi_pe-s43");
        assert_eq!(
            after_boundary,
            "20260506-com.google.chrome-docs_screenpi_pe-s44"
        );
    }

    #[test]
    fn timezone_is_resolved_at_this_boundary_not_in_the_engine() {
        let utc = SessionIdentityDeriver::new(FixedOffsetClock { offset_minutes: 0 });
        let tokyo = SessionIdentityDeriver::new(FixedOffsetClock {
            offset_minutes: 9 * 60,
        });
        let url = Some("https://docs.screenpi.pe/architecture");

        assert_eq!(
            utc.derive(context("Deep Dive", url, BASE_MS))
                .unwrap()
                .session_id,
            "20260507-com.google.chrome-docs_screenpi_pe-s09"
        );
        assert_eq!(
            tokyo
                .derive(context("Deep Dive", url, BASE_MS))
                .unwrap()
                .session_id,
            "20260507-com.google.chrome-docs_screenpi_pe-s27"
        );
    }

    #[test]
    fn an_unresolvable_clock_is_a_typed_state_not_an_invented_session() {
        struct NoClock;
        impl LocalClock for NoClock {
            fn local_civil_time(&self, _unix_ms: i64) -> Option<LocalCivilTime> {
                None
            }
        }

        assert_eq!(
            SessionIdentityDeriver::new(NoClock)
                .derive(context("Deep Dive", None, BASE_MS))
                .unwrap_err(),
            SessionIdentityUnavailable::LocalTimeUnavailable
        );
    }

    #[test]
    fn the_platform_clock_resolves_a_usable_civil_time() {
        let civil = SystemLocalClock
            .local_civil_time(BASE_MS)
            .expect("the host timezone database must resolve a real instant");
        assert_eq!(civil.day().len(), 8);
        assert!(
            civil.day().starts_with("202605"),
            "every real UTC offset keeps this instant inside May 2026: {}",
            civil.day()
        );
        assert!(civil.minute_of_day < 24 * 60);
    }
}
