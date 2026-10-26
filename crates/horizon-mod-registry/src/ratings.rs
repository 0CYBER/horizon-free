//! Players' ratings of published projects, 1 to 5 stars: one per GitHub
//! account and project, which its rater can change. Stored in one SQLite
//! file (a Railway volume). Only totals leave it: never who rated what,
//! except a signed-in account's own rating, to that account.
use rusqlite::{Connection, OptionalExtension, params};
use std::{collections::BTreeMap, path::Path};

const SCHEMA: &str = "
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
CREATE TABLE IF NOT EXISTS ratings (
    project TEXT NOT NULL,
    account INTEGER NOT NULL,
    stars INTEGER NOT NULL CHECK (stars BETWEEN 1 AND 5),
    updated INTEGER NOT NULL,
    PRIMARY KEY (project, account)
) WITHOUT ROWID;
";

/// A project's ratings: their mean, and how many.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Summary {
    pub average: f64,
    pub count: u64,
}

pub struct Ratings {
    db: Connection,
}

impl Ratings {
    /// Opens (or creates) the file at `path`; `:memory:` for a test.
    pub fn open(path: &Path) -> rusqlite::Result<Ratings> {
        let db = Connection::open(path)?;
        db.execute_batch(SCHEMA)?;
        Ok(Ratings { db })
    }

    /// Sets `account`'s rating of `project`, replacing an earlier one.
    pub fn rate(&self, project: &str, account: u64, stars: u8, now: u64) -> rusqlite::Result<()> {
        self.db.execute(
            "INSERT INTO ratings (project, account, stars, updated) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (project, account) DO UPDATE SET stars = excluded.stars, updated = excluded.updated",
            params![project, account as i64, stars, now as i64],
        )?;
        Ok(())
    }

    /// `account`'s own rating of `project`.
    pub fn mine(&self, project: &str, account: u64) -> rusqlite::Result<Option<u8>> {
        self.db
            .query_row(
                "SELECT stars FROM ratings WHERE project = ?1 AND account = ?2",
                params![project, account as i64],
                |r| r.get(0),
            )
            .optional()
    }

    /// `project`'s summary; None when nobody rated it.
    pub fn summary(&self, project: &str) -> rusqlite::Result<Option<Summary>> {
        self.db
            .query_row(
                "SELECT avg(stars), count(*) FROM ratings WHERE project = ?1 HAVING count(*) > 0",
                [project],
                |r| {
                    Ok(Summary {
                        average: r.get(0)?,
                        count: r.get::<_, i64>(1)? as u64,
                    })
                },
            )
            .optional()
    }

    /// Every rated project's summary.
    pub fn summaries(&self) -> rusqlite::Result<BTreeMap<String, Summary>> {
        let mut statement = self
            .db
            .prepare("SELECT project, avg(stars), count(*) FROM ratings GROUP BY project")?;
        let rows = statement.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                Summary {
                    average: r.get(1)?,
                    count: r.get::<_, i64>(2)? as u64,
                },
            ))
        })?;
        rows.collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_rating_per_account_and_project() {
        let r = Ratings::open(Path::new(":memory:")).unwrap();
        r.rate("zoom", 1, 5, 10).unwrap();
        r.rate("zoom", 2, 2, 10).unwrap();
        r.rate("zoom", 1, 4, 11).unwrap();
        r.rate("minimap", 1, 3, 11).unwrap();
        assert_eq!(r.mine("zoom", 1).unwrap(), Some(4));
        assert_eq!(r.mine("zoom", 3).unwrap(), None);
        let all = r.summaries().unwrap();
        assert_eq!(
            all["zoom"],
            Summary {
                average: 3.0,
                count: 2
            }
        );
        assert_eq!(
            all["minimap"],
            Summary {
                average: 3.0,
                count: 1
            }
        );
        assert_eq!(r.summary("zoom").unwrap(), Some(all["zoom"]));
        assert_eq!(r.summary("nothing").unwrap(), None);
        assert!(
            r.rate("zoom", 1, 6, 12).is_err(),
            "the table checks the range too"
        );
    }
}
