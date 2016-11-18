// This file is released under the same terms as Rust itself.

use ci::CiId;
use db::{self, CiState, Db, PendingEntry, QueueEntry, RunningEntry};
use hyper::Url;
use pipeline::PipelineId;
use rusqlite::{self, Connection};
use std::convert::AsRef;
use std::error::Error;
use std::path::Path;
use ui::Pr;
use vcs::Commit;

pub struct SqliteDb {
    conn: Connection,
}

impl SqliteDb {
    pub fn open<Q: AsRef<Path>>(path: Q) -> rusqlite::Result<Self> {
        let conn = try!(Connection::open(path));
        try!(conn.execute_batch(r###"
            CREATE TABLE IF NOT EXISTS ci_state (
                ci_id INTEGER,
                ci_state INTEGER,
                ci_commit STRING
            );
            CREATE TABLE IF NOT EXISTS queue (
                id INTEGER PRIMARY KEY,
                pipeline_id INTEGER,
                pr TEXT,
                message TEXT,
                pull_commit TEXT
            );
            CREATE TABLE IF NOT EXISTS running (
                pipeline_id INTEGER PRIMARY KEY,
                pr TEXT,
                message TEXT,
                pull_commit TEXT,
                merge_commit TEXT,
                canceled INT,
                built INT
            );
            CREATE TABLE IF NOT EXISTS pending (
                id INTEGER PRIMARY KEY,
                pipeline_id INTEGER,
                pr TEXT,
                pull_commit TEXT,
                title TEXT,
                url TEXT
            );
        "###));
        Ok(SqliteDb{
            conn: conn,
        })
    }
}


impl Db for SqliteDb {
    fn transaction<T: db::Transaction>(
        &mut self,
        t: T,
    ) -> Result<T::Return, Box<Error + Send + Sync>> {
        let mut transaction = SqliteTransaction::new(
            try!(self.conn.transaction())
        );
        let return_ = try!(t.run(&mut transaction));
        try!(transaction.conn.commit());
        Ok(return_)
    }
    fn push_queue(
        &mut self,
        pipeline_id: PipelineId,
        queue_entry: QueueEntry,
    ) -> Result<(), Box<Error + Send + Sync>> {
        SqliteTransaction::new(
            try!(self.conn.transaction())
        ).push_queue(pipeline_id, queue_entry)
    }
    fn pop_queue(
        &mut self,
        pipeline_id: PipelineId,
    ) -> Result<Option<QueueEntry>, Box<Error + Send + Sync>> {
        SqliteTransaction::new(
            try!(self.conn.transaction())
        ).pop_queue(pipeline_id)
    }
    fn list_queue(
        &mut self,
        pipeline_id: PipelineId,
    ) -> Result<Vec<QueueEntry>, Box<Error + Send + Sync>> {
        SqliteTransaction::new(
            try!(self.conn.transaction())
        ).list_queue(pipeline_id)
    }
    fn put_running(
        &mut self,
        pipeline_id: PipelineId,
        running_entry: RunningEntry,
    ) -> Result<(), Box<Error + Send + Sync>> {
        SqliteTransaction::new(
            try!(self.conn.transaction())
        ).put_running(pipeline_id, running_entry)
    }
    fn take_running(
        &mut self,
        pipeline_id: PipelineId,
    ) -> Result<Option<RunningEntry>, Box<Error + Send + Sync>> {
        SqliteTransaction::new(
            try!(self.conn.transaction())
        ).take_running(pipeline_id)
    }
    fn peek_running(
        &mut self,
        pipeline_id: PipelineId,
    ) -> Result<Option<RunningEntry>, Box<Error + Send + Sync>> {
        SqliteTransaction::new(
            try!(self.conn.transaction())
        ).peek_running(pipeline_id)
    }
    fn add_pending(
        &mut self,
        pipeline_id: PipelineId,
        entry: PendingEntry,
    ) -> Result<(), Box<Error + Send + Sync>> {
        SqliteTransaction::new(
            try!(self.conn.transaction())
        ).add_pending(pipeline_id, entry)
    }
    fn take_pending_by_pr(
        &mut self,
        pipeline_id: PipelineId,
        pr: &Pr,
    ) -> Result<Option<PendingEntry>, Box<Error + Send + Sync>> {
        SqliteTransaction::new(
            try!(self.conn.transaction())
        ).take_pending_by_pr(pipeline_id, pr)
    }
    fn peek_pending_by_pr(
        &mut self,
        pipeline_id: PipelineId,
        pr: &Pr,
    ) -> Result<Option<PendingEntry>, Box<Error + Send + Sync>> {
        SqliteTransaction::new(
            try!(self.conn.transaction())
        ).peek_pending_by_pr(pipeline_id, pr)
    }
    fn list_pending(
        &mut self,
        pipeline_id: PipelineId,
    ) -> Result<Vec<PendingEntry>, Box<Error + Send + Sync>> {
        SqliteTransaction::new(
            try!(self.conn.transaction())
        ).list_pending(pipeline_id)
    }
    fn cancel_by_pr(
        &mut self,
        pipeline_id: PipelineId,
        pr: &Pr,
    ) -> Result<(), Box<Error + Send + Sync>> {
        SqliteTransaction::new(
            try!(self.conn.transaction())
        ).cancel_by_pr(pipeline_id, pr)
    }
    fn cancel_by_pr_different_commit(
        &mut self,
        pipeline_id: PipelineId,
        pr: &Pr,
        commit: &Commit,
    ) -> Result<bool, Box<Error + Send + Sync>> {
        SqliteTransaction::new(
            try!(self.conn.transaction())
        ).cancel_by_pr_different_commit(pipeline_id, pr, commit)
    }
    fn set_ci_state(
        &mut self,
        ci_id: CiId,
        ci_state: CiState,
        commit: &Commit,
    ) -> Result<(), Box<Error + Send + Sync>> {
        SqliteTransaction::new(
            try!(self.conn.transaction())
        ).set_ci_state(ci_id, ci_state, commit)
    }
    fn clear_ci_state(
        &mut self,
        ci_id: CiId,
    ) -> Result<(), Box<Error + Send + Sync>> {
        SqliteTransaction::new(
            try!(self.conn.transaction())
        ).clear_ci_state(ci_id)
    }
    fn get_ci_state(
        &mut self,
        ci_id: CiId,
    ) -> Result<Option<(CiState, Commit)>, Box<Error + Send + Sync>> {
        SqliteTransaction::new(
            try!(self.conn.transaction())
        ).get_ci_state(ci_id)
    }
}


pub struct SqliteTransaction<'a> {
    conn: rusqlite::Transaction<'a>,
}

impl<'a> SqliteTransaction<'a> {
    pub fn new(conn: rusqlite::Transaction<'a>) -> Self {
        SqliteTransaction {
            conn: conn,
        }
    }
}

impl<'a> Db for SqliteTransaction<'a> {
    fn push_queue(
        &mut self,
        pipeline_id: PipelineId,
        QueueEntry{pr, commit, message}: QueueEntry,
    ) -> Result<(), Box<Error + Send + Sync>> {
        let sql = r###"
            INSERT INTO queue (pr, pipeline_id, pull_commit, message)
            VALUES (?, ?, ?, ?)
        "###;
        try!(self.conn.execute(sql, &[
            &pr.as_str(),
            &pipeline_id.0,
            &commit.as_str(),
            &message,
        ]));
        Ok(())
    }
    fn pop_queue(
        &mut self,
        pipeline_id: PipelineId,
    ) -> Result<Option<QueueEntry>, Box<Error + Send + Sync>> {
        let sql = r###"
            SELECT id, pr, pull_commit, message
            FROM queue
            WHERE pipeline_id = ?
            ORDER BY id ASC LIMIT 1
        "###;
        let item = {
            let mut stmt = try!(self.conn.prepare(sql));
            let mut rows = try!(stmt
            .query_map(&[&pipeline_id.0], |row| (
                row.get::<_, i32>(0),
                QueueEntry {
                    pr: Pr::from(row.get::<_, String>(1)),
                    commit: Commit::from(row.get::<_, String>(2)),
                    message: row.get::<_, String>(3),
                },
            )));
            rows.next()
        };
        if let Some(Ok((id, item))) = item {
            let sql = r###"
                DELETE FROM queue WHERE id = ?
            "###;
            try!(self.conn.execute(sql, &[&id]));
            Ok(Some(item))
        } else if let Some(Err(e)) = item {
            Err(e.into())
        } else {
            Ok(None)
        }
    }
    fn list_queue(
        &mut self,
        pipeline_id: PipelineId,
    ) -> Result<Vec<QueueEntry>, Box<Error + Send + Sync>> {
        let sql = r###"
            SELECT pr, pull_commit, message
            FROM queue
            WHERE pipeline_id = ?
            ORDER BY id ASC
        "###;
        let mut stmt = try!(self.conn.prepare(&sql));
        let rows = try!(stmt.query_map(&[&pipeline_id.0], |row| QueueEntry {
                pr: Pr::from(row.get::<_, String>(0)),
                commit: Commit::from(row.get::<_, String>(1)),
                message: row.get::<_, String>(2),
            })
        );
        let mut v = vec![];
        for item in rows {
            match item {
                Ok(item) => v.push(item),
                Err(e) => return Err(e.into()),
            }
        }
        Ok(v)
    }
    fn put_running(
        &mut self,
        pipeline_id: PipelineId,
        RunningEntry{
            pr,
            pull_commit,
            merge_commit,
            message,
            canceled,
            built,
        }: RunningEntry
    ) -> Result<(), Box<Error + Send + Sync>> {
        let sql = r###"
            REPLACE INTO running
                (
                    pipeline_id,
                    pr,
                    pull_commit,
                    merge_commit,
                    message,
                    canceled,
                    built
                )
            VALUES
                (?, ?, ?, ?, ?, ?, ?)
        "###;
        try!(self.conn.execute(sql, &[
            &pipeline_id.0,
            &pr.as_str(),
            &pull_commit.as_str(),
            &merge_commit.as_ref().map(Commit::as_str),
            &message,
            &canceled,
            &built,
        ]));
        Ok(())
    }
    fn take_running(
        &mut self,
        pipeline_id: PipelineId,
    ) -> Result<Option<RunningEntry>, Box<Error + Send + Sync>> {
        let sql = r###"
            SELECT pr, pull_commit, merge_commit, message, canceled, built
            FROM running
            WHERE pipeline_id = ?
        "###;
        let entry = {
            let mut stmt = try!(self.conn.prepare(&sql));
            let mut rows = try!(stmt
                .query_map(&[&pipeline_id.0], |row| RunningEntry {
                    pr: Pr::from(row.get::<_, String>(0)),
                    pull_commit: Commit::from(row.get::<_, String>(1)),
                    merge_commit: row.get::<_, Option<String>>(2)
                        .map(Commit::from),
                    message: row.get(3),
                    canceled: row.get(4),
                    built: row.get(5),
                })
            );
            match rows.next() {
                Some(Err(e)) => return Err(e.into()),
                Some(Ok(item)) => Some(item),
                None => None,
            }
        };
        let sql = r###"
            DELETE FROM running WHERE pipeline_id = ?
        "###;
        try!(self.conn.execute(sql, &[&pipeline_id.0]));
        Ok(entry)
    }
    fn peek_running(
        &mut self,
        pipeline_id: PipelineId,
    ) -> Result<Option<RunningEntry>, Box<Error + Send + Sync>> {
        let sql = r###"
            SELECT pr, pull_commit, merge_commit, message, canceled, built
            FROM running
            WHERE pipeline_id = ?
        "###;
        let mut stmt = try!(self.conn.prepare(&sql));
        let mut rows = try!(stmt
            .query_map(&[&pipeline_id.0], |row| RunningEntry {
                pr: Pr::from(row.get::<_, String>(0)),
                pull_commit: Commit::from(row.get::<_, String>(1)),
                merge_commit: row.get::<_, Option<String>>(2)
                    .map(Commit::from),
                message: row.get(3),
                canceled: row.get(4),
                built: row.get(5),
            })
        );
        match rows.next() {
            Some(Err(e)) => Err(e.into()),
            Some(Ok(item)) => Ok(Some(item)),
            None => Ok(None),
        }
    }
    fn add_pending(
        &mut self,
        pipeline_id: PipelineId,
        entry: PendingEntry,
    ) -> Result<(), Box<Error + Send + Sync>> {
        let sql = r###"
            DELETE FROM pending WHERE pipeline_id = ? AND pr = ?
        "###;
        self.conn.execute(sql, &[
            &pipeline_id.0,
            &entry.pr.as_str(),
        ]).expect("Remove pending entry");
        let sql = r###"
            INSERT INTO pending (pipeline_id, pr, pull_commit, title, url)
            VALUES (?, ?, ?, ?, ?)
        "###;
        try!(self.conn.execute(sql, &[
            &pipeline_id.0,
            &entry.pr.as_str(),
            &entry.commit.as_str(),
            &entry.title,
            &entry.url.as_str(),
        ]));
        Ok(())
    }
    fn take_pending_by_pr(
        &mut self,
        pipeline_id: PipelineId,
        pr: &Pr,
    ) -> Result<Option<PendingEntry>, Box<Error + Send + Sync>> {
        let sql = r###"
            SELECT id, pr, pull_commit, title, url
            FROM pending
            WHERE pipeline_id = ? AND pr = ?
        "###;
        let entry = {
            let mut stmt = try!(self.conn.prepare(&sql));
            let mut rows = try!(stmt
                .query_map(&[
                    &pipeline_id.0,
                    &pr.as_str(),
                ], |row| (row.get::<_, i64>(0), PendingEntry {
                    pr: Pr::from(row.get::<_, String>(1)),
                    commit: Commit::from(row.get::<_, String>(2)),
                    title: row.get(3),
                    url: Url::parse(&row.get::<_, String>(4)).unwrap(),
                }))
            );
            match rows.next() {
                Some(Err(e)) => return Err(e.into()),
                Some(Ok(item)) => Some(item),
                None => None,
            }
        };
        if let Some((id, item)) = entry {
            let sql = r###"
                DELETE FROM queue WHERE id = ?
            "###;
            try!(self.conn.execute(sql, &[&id]));
            Ok(Some(item))
        } else {
            Ok(None)
        }
    }
    fn peek_pending_by_pr(
        &mut self,
        pipeline_id: PipelineId,
        pr: &Pr,
    ) -> Result<Option<PendingEntry>, Box<Error + Send + Sync>> {
        let sql = r###"
            SELECT pr, pull_commit, title, url
            FROM pending
            WHERE pipeline_id = ? AND pr = ?
        "###;
        let mut stmt = try!(self.conn.prepare(&sql));
        let mut rows = try!(stmt
            .query_map(&[
                &pipeline_id.0,
                &pr.as_str(),
            ], |row| PendingEntry {
                pr: Pr::from(row.get::<_, String>(0)),
                commit: Commit::from(row.get::<_, String>(1)),
                title: row.get(2),
                url: Url::parse(&row.get::<_, String>(3)).unwrap(),
            })
        );
        match rows.next() {
            Some(Err(e)) => Err(e.into()),
            Some(Ok(item)) => Ok(Some(item)),
            None => Ok(None),
        }
    }
    fn list_pending(
        &mut self,
        pipeline_id: PipelineId,
    ) -> Result<Vec<PendingEntry>, Box<Error + Send + Sync>> {
        let sql = r###"
            SELECT pr, pull_commit, title, url
            FROM pending
            WHERE pipeline_id = ?
        "###;
        let mut stmt = try!(self.conn.prepare(&sql));
        let rows = try!(stmt.query_map(&[&pipeline_id.0], |row| PendingEntry {
                pr: Pr::from(row.get::<_, String>(0)),
                commit: Commit::from(row.get::<_, String>(1)),
                title: row.get(2),
                url: Url::parse(&row.get::<_, String>(3)).unwrap(),
            })
        );
        let mut v = vec![];
        for item in rows {
            match item {
                Ok(item) => v.push(item),
                Err(e) => return Err(e.into()),
            }
        }
        Ok(v)
    }
    fn cancel_by_pr(
        &mut self,
        pipeline_id: PipelineId,
        pr: &Pr,
    ) -> Result<(), Box<Error + Send + Sync>> {
        let sql = r###"
            UPDATE running
            SET canceled = 1
            WHERE pipeline_id = ? AND pr = ?
        "###;
        try!(self.conn.execute(sql, &[
            &pipeline_id.0,
            &pr.as_str(),
        ]));
        let sql = r###"
            DELETE FROM queue
            WHERE pipeline_id = ? AND pr = ?
        "###;
        try!(self.conn.execute(sql, &[
            &pipeline_id.0,
            &pr.as_str(),
        ]));
        Ok(())
    }
    fn cancel_by_pr_different_commit(
        &mut self,
        pipeline_id: PipelineId,
        pr: &Pr,
        commit: &Commit,
    ) -> Result<bool, Box<Error + Send + Sync>> {
        let sql = r###"
            UPDATE running
            SET canceled = 1
            WHERE pipeline_id = ? AND pr = ? AND pull_commit <> ?
        "###;
        let affected_rows_running = try!(self.conn.execute(sql, &[
            &pipeline_id.0,
            &pr.as_str(),
            &commit.as_str(),
        ]));
        let sql = r###"
            DELETE FROM queue
            WHERE pipeline_id = ? AND pr = ? AND pull_commit <> ?
        "###;
        let affected_rows_queue = try!(self.conn.execute(sql, &[
            &pipeline_id.0,
            &pr.as_str(),
            &commit.as_str(),
        ]));
        Ok(affected_rows_queue != 0 || affected_rows_running != 0)
    }
    fn set_ci_state(
        &mut self,
        ci_id: CiId,
        ci_state: CiState,
        commit: &Commit,
    ) -> Result<(), Box<Error + Send + Sync>> {
        let sql = r###"
            REPLACE INTO ci_state
                (
                    ci_id,
                    ci_state,
                    ci_commit
                )
            VALUES
                (?, ?, ?)
        "###;
        try!(self.conn.execute(sql, &[
            &ci_id.0,
            &(ci_state as i32),
            &commit.as_str(),
        ]));
        Ok(())
    }
    fn clear_ci_state(
        &mut self,
        ci_id: CiId,
    ) -> Result<(), Box<Error + Send + Sync>> {
        let sql = r###"
            DELETE FROM ci_state
            WHERE ci_id = ?
        "###;
        try!(self.conn.execute(sql, &[
            &ci_id.0,
        ]));
        Ok(())
    }
    fn get_ci_state(
        &mut self,
        ci_id: CiId,
    ) -> Result<Option<(CiState, Commit)>, Box<Error + Send + Sync>> {
        let sql = r###"
            SELECT ci_state, ci_commit
            FROM ci_state
            WHERE ci_id = ?
        "###;
        let entry = {
            let mut stmt = try!(self.conn.prepare(&sql));
            let mut rows = try!(stmt
                .query_map(&[
                    &ci_id.0,
                ], |row| (
                    CiState::from_i32(row.get::<_, i32>(0)),
                    Commit::from(row.get::<_, String>(1)),
                ))
            );
            match rows.next() {
                Some(Err(e)) => return Err(e.into()),
                Some(Ok(item)) => Some(item),
                None => None,
            }
        };
        Ok(entry)
    }
}