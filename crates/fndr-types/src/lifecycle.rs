//! Lifecycle enums persisted as integer discriminants (T-201, invariant:
//! never strings). v1 stored lifecycle as free-form strings ("" / "pending" /
//! "reviewed_local" / ...), which drifted across three schemas. Rules here:
//! discriminant values are append-only, never renumbered, never reused.

/// Post-capture review lifecycle of a memory record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i64)]
pub enum ReviewLifecycle {
    /// Persisted, not yet queued for review.
    Captured = 0,
    /// In the durable review queue.
    Pending = 1,
    /// Per-record local review succeeded.
    ReviewedLocal = 2,
    /// Consolidated by the daily pass.
    ReviewedDaily = 3,
    /// Review attempts exhausted; surfaced, never silent.
    ReviewFailed = 4,
}

/// Task state for the tasks surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i64)]
pub enum TaskStatus {
    Open = 0,
    Done = 1,
    Dismissed = 2,
}

/// Conversion error carrying the offending value for diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownDiscriminant {
    pub type_name: &'static str,
    pub value: i64,
}

impl std::fmt::Display for UnknownDiscriminant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "unknown {} discriminant: {}", self.type_name, self.value)
    }
}

impl std::error::Error for UnknownDiscriminant {}

macro_rules! discriminant_conversions {
    ($ty:ident { $($variant:ident),+ $(,)? }) => {
        impl From<$ty> for i64 {
            fn from(value: $ty) -> i64 {
                value as i64
            }
        }
        impl TryFrom<i64> for $ty {
            type Error = UnknownDiscriminant;
            fn try_from(value: i64) -> Result<Self, Self::Error> {
                $(
                    if value == $ty::$variant as i64 {
                        return Ok($ty::$variant);
                    }
                )+
                Err(UnknownDiscriminant {
                    type_name: stringify!($ty),
                    value,
                })
            }
        }
        impl $ty {
            pub const ALL: &'static [$ty] = &[$($ty::$variant),+];
        }
    };
}

discriminant_conversions!(ReviewLifecycle {
    Captured,
    Pending,
    ReviewedLocal,
    ReviewedDaily,
    ReviewFailed,
});
discriminant_conversions!(TaskStatus {
    Open,
    Done,
    Dismissed
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_round_trips_every_variant() {
        for variant in ReviewLifecycle::ALL {
            let raw: i64 = (*variant).into();
            assert_eq!(ReviewLifecycle::try_from(raw).unwrap(), *variant);
        }
        for variant in TaskStatus::ALL {
            let raw: i64 = (*variant).into();
            assert_eq!(TaskStatus::try_from(raw).unwrap(), *variant);
        }
    }

    #[test]
    fn unknown_discriminant_is_a_typed_error() {
        let err = ReviewLifecycle::try_from(999).unwrap_err();
        assert_eq!(err.value, 999);
        assert_eq!(err.type_name, "ReviewLifecycle");
    }

    #[test]
    fn discriminant_values_are_pinned() {
        // These values are on disk; a renumbering must fail loudly here.
        assert_eq!(ReviewLifecycle::Captured as i64, 0);
        assert_eq!(ReviewLifecycle::Pending as i64, 1);
        assert_eq!(ReviewLifecycle::ReviewedLocal as i64, 2);
        assert_eq!(ReviewLifecycle::ReviewedDaily as i64, 3);
        assert_eq!(ReviewLifecycle::ReviewFailed as i64, 4);
        assert_eq!(TaskStatus::Open as i64, 0);
        assert_eq!(TaskStatus::Done as i64, 1);
        assert_eq!(TaskStatus::Dismissed as i64, 2);
    }
}
