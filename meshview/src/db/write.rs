// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! Writing, on either database, with one SQL text.
//!
//! What the ingester and the jobs write is plain SQL both databases read the
//! same way, so it is written once, with `?` for its values, and numbered as
//! `$1`, `$2`… for PostgreSQL when it runs. The values go as [`Arg`]s, which
//! bind the same on either. A statement that cannot be written for both is not
//! for here: it belongs in the database's own module.

use super::Pool;
use anyhow::Result;
use sqlx::{Sqlite, Transaction};

/// A value for a `?` in a statement.
#[derive(Debug, Clone, Copy)]
pub enum Arg<'a> {
    Int(i64),
    OptInt(Option<i64>),
    Text(&'a str),
    OptText(Option<&'a str>),
    Bytes(&'a [u8]),
    OptReal(Option<f64>),
    Bool(bool),
}

/// A transaction on whichever database the pool is on.
pub enum Tx<'c> {
    Sqlite(Transaction<'c, Sqlite>),
    #[cfg(feature = "postgres")]
    Postgres(Transaction<'c, sqlx::Postgres>),
}

/// Binds the arguments in order, on a query of either database.
macro_rules! bind {
    ($query:expr, $args:expr) => {{
        let mut query = $query;
        for arg in $args {
            query = match *arg {
                Arg::Int(value) => query.bind(value),
                Arg::OptInt(value) => query.bind(value),
                Arg::Text(value) => query.bind(value),
                Arg::OptText(value) => query.bind(value),
                Arg::Bytes(value) => query.bind(value),
                Arg::OptReal(value) => query.bind(value),
                Arg::Bool(value) => query.bind(value),
            };
        }
        query
    }};
}

/// `sql` with each `?` numbered as PostgreSQL wants it. The statements here
/// have no `?` but those.
#[cfg(feature = "postgres")]
fn numbered(sql: &str) -> String {
    let mut out = String::with_capacity(sql.len() + 16);
    let mut n = 0;
    for c in sql.chars() {
        if c == '?' {
            n += 1;
            out.push('$');
            out.push_str(&n.to_string());
        } else {
            out.push(c);
        }
    }
    out
}

/// Starts a write. In SQLite, taking the write lock at once, so that a
/// transaction never has to give up halfway for another writer.
pub async fn begin(pool: &Pool) -> Result<Tx<'static>> {
    Ok(match pool {
        Pool::Sqlite(pool) => Tx::Sqlite(pool.begin_with("BEGIN IMMEDIATE").await?),
        #[cfg(feature = "postgres")]
        Pool::Postgres(pool) => Tx::Postgres(pool.begin().await?),
    })
}

impl Tx<'_> {
    /// Runs a statement, and answers how many rows it changed.
    pub async fn execute(&mut self, sql: &'static str, args: &[Arg<'_>]) -> Result<u64> {
        Ok(match self {
            Self::Sqlite(tx) => bind!(sqlx::query(sql), args)
                .execute(&mut **tx)
                .await?
                .rows_affected(),
            #[cfg(feature = "postgres")]
            Self::Postgres(tx) => bind!(sqlx::query(sqlx::AssertSqlSafe(numbered(sql))), args)
                .execute(&mut **tx)
                .await?
                .rows_affected(),
        })
    }

    /// Runs a statement that has to be written differently for each database,
    /// with the same values, and answers how many rows it changed.
    pub async fn execute_either(
        &mut self,
        sqlite: &'static str,
        postgres: &'static str,
        args: &[Arg<'_>],
    ) -> Result<u64> {
        #[cfg(not(feature = "postgres"))]
        let _ = postgres;
        Ok(match self {
            Self::Sqlite(tx) => bind!(sqlx::query(sqlite), args)
                .execute(&mut **tx)
                .await?
                .rows_affected(),
            #[cfg(feature = "postgres")]
            Self::Postgres(tx) => bind!(sqlx::query(sqlx::AssertSqlSafe(numbered(postgres))), args)
                .execute(&mut **tx)
                .await?
                .rows_affected(),
        })
    }

    pub async fn commit(self) -> Result<()> {
        match self {
            Self::Sqlite(tx) => tx.commit().await?,
            #[cfg(feature = "postgres")]
            Self::Postgres(tx) => tx.commit().await?,
        }
        Ok(())
    }
}

/// Runs one statement on its own, in a transaction of its own.
pub async fn execute(pool: &Pool, sql: &'static str, args: &[Arg<'_>]) -> Result<u64> {
    let mut tx = begin(pool).await?;
    let changed = tx.execute(sql, args).await?;
    tx.commit().await?;
    Ok(changed)
}

/// Runs a statement that takes no values and cannot run in a transaction,
/// such as VACUUM, as it is.
pub async fn run(pool: &Pool, sql: &'static str) -> Result<()> {
    match pool {
        Pool::Sqlite(pool) => {
            sqlx::raw_sql(sql).execute(pool).await?;
        }
        #[cfg(feature = "postgres")]
        Pool::Postgres(pool) => {
            sqlx::raw_sql(sql).execute(pool).await?;
        }
    }
    Ok(())
}

/// The first column of every row a query answers, as integers, outside any
/// transaction.
pub async fn integers(pool: &Pool, sql: &'static str, args: &[Arg<'_>]) -> Result<Vec<i64>> {
    Ok(match pool {
        Pool::Sqlite(pool) => bind!(sqlx::query_scalar(sql), args).fetch_all(pool).await?,
        #[cfg(feature = "postgres")]
        Pool::Postgres(pool) => {
            bind!(sqlx::query_scalar(sqlx::AssertSqlSafe(numbered(sql))), args)
                .fetch_all(pool)
                .await?
        }
    })
}

#[cfg(all(test, feature = "postgres"))]
mod tests {
    use super::numbered;

    #[test]
    fn placeholders_are_numbered_in_order() {
        assert_eq!(
            numbered("UPDATE t SET a = ?, b = COALESCE(?, b) WHERE c = ?"),
            "UPDATE t SET a = $1, b = COALESCE($2, b) WHERE c = $3"
        );
    }
}
