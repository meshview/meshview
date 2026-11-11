// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! The database, and the line between what is true of any of them and what is
//! true of SQLite.
//!
//! Only SQLite exists for now. Everything that knows it is talking to SQLite —
//! connection options, its catalogue, its migrations — lives in [`sqlite`], so
//! that PostgreSQL can arrive as a sibling module rather than as a rewrite. What
//! sits here is what the rest of the program reasons about whichever database it
//! is.

pub mod sqlite;

use std::fmt;

/// The last Alembic revision of the Python meshview, and the only one this
/// program agrees to take a database over from. A database at an earlier one
/// has to be brought up to date by the Python version first.
pub const PYTHON_HEAD_REVISION: &str = "44612e74a798";

/// What a database holds, as far as its schema goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Schema {
    /// Nothing at all: a new file, about to be built from scratch.
    Empty,
    /// Created by the Python meshview and not yet adopted, at this Alembic
    /// revision.
    Python { revision: String },
    /// Already ours, with these migrations applied, in order.
    Migrated { applied: Vec<i64> },
    /// Tables, but neither ours nor Python's bookkeeping. Not something to write
    /// over.
    Unknown,
}

/// How a database's schema compares with what this build expects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Compatibility {
    /// Exactly what this build was written against.
    Current,
    /// Not there yet. A writer would migrate it; a reader waits for that.
    Behind,
    /// Migrated by a newer build than this one, which this one cannot be trusted
    /// to understand.
    Ahead { latest_known: i64, found: i64 },
    /// Something that cannot become current by migrating, and why.
    Incompatible(String),
}

impl Schema {
    /// Compares this schema with the migrations a build carries.
    pub fn compatibility(&self, known: &[i64]) -> Compatibility {
        let latest_known = known.last().copied().unwrap_or(0);
        match self {
            Self::Empty => Compatibility::Behind,
            Self::Python { revision } if revision == PYTHON_HEAD_REVISION => Compatibility::Behind,
            Self::Python { revision } => Compatibility::Incompatible(format!(
                "this database is at the Python meshview's Alembic revision {revision}, and \
                 meshview-rs only takes over databases at {PYTHON_HEAD_REVISION}. Run the \
                 Python meshview 4.0 against it once so that it migrates, then try again."
            )),
            Self::Migrated { applied } => {
                let found = applied.last().copied().unwrap_or(0);
                if found > latest_known {
                    Compatibility::Ahead {
                        latest_known,
                        found,
                    }
                } else if applied.as_slice() == known {
                    Compatibility::Current
                } else {
                    Compatibility::Behind
                }
            }
            Self::Unknown => Compatibility::Incompatible(
                "this database has tables but was not created by meshview: refusing to touch it"
                    .to_string(),
            ),
        }
    }
}

impl fmt::Display for Schema {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "empty"),
            Self::Python { revision } => write!(f, "Python meshview at Alembic {revision}"),
            Self::Migrated { applied } => match applied.last() {
                Some(last) => write!(f, "meshview-rs at migration {last}"),
                None => write!(f, "meshview-rs with no migrations applied"),
            },
            Self::Unknown => write!(f, "not a meshview database"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KNOWN: &[i64] = &[1, 2];

    #[test]
    fn only_the_last_python_revision_is_taken_over() {
        let head = Schema::Python {
            revision: PYTHON_HEAD_REVISION.into(),
        };
        let older = Schema::Python {
            revision: "4d2a7c9e1b30".into(),
        };

        assert_eq!(head.compatibility(KNOWN), Compatibility::Behind);
        assert!(matches!(
            older.compatibility(KNOWN),
            Compatibility::Incompatible(_)
        ));
    }

    #[test]
    fn migrated_databases_are_compared_with_what_this_build_knows() {
        let at = |applied: &[i64]| Schema::Migrated {
            applied: applied.to_vec(),
        };

        assert_eq!(at(&[1, 2]).compatibility(KNOWN), Compatibility::Current);
        assert_eq!(at(&[1]).compatibility(KNOWN), Compatibility::Behind);
        assert_eq!(
            at(&[1, 2, 3]).compatibility(KNOWN),
            Compatibility::Ahead {
                latest_known: 2,
                found: 3
            }
        );
        assert_eq!(Schema::Empty.compatibility(KNOWN), Compatibility::Behind);
        assert!(matches!(
            Schema::Unknown.compatibility(KNOWN),
            Compatibility::Incompatible(_)
        ));
    }
}
