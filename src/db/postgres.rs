// This file is released under the same terms as Rust itself.

use ci::CiId;
use db::{self, CiState, Db, PendingEntry, QueueEntry, RunningEntry};
use hyper::Url;
use postgres::{Connection, TlsMode};
use postgres::params::{ConnectParams, IntoConnectParams};
use postgres::transaction::Transaction as PgTransaction;
use std::error::Error;
use ui::Pr;
use pipeline::PipelineId;
use vcs::Commit;

pub struct PostgresDb {
    params: ConnectParams,
}

impl PostgresDb {
    pub fn open<Q: IntoConnectParams>(
        params: Q
    ) -> Result<Self, Box<Error + Send + Sync>> {
        let result = PostgresDb{
            params: try!(params.into_connect_params()),
        };
        try!(try!(result.conn()).batch_execute(r###"
            CREATE TABLE IF NOT EXISTS ci_state (
                ci_id SERIAL PRIMARY KEY,
                ci_state INTEGER,
                ci_commit TEXT
            );
            CREATE TABLE IF NOT EXISTS queue (
                id SERIAL PRIMARY KEY,
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
                canceled BOOLEAN,
                built BOOLEAN
            );
            CREATE TABLE IF NOT EXISTS pending (
                id SERIAL PRIMARY KEY,
                pipeline_id INTEGER,
                pr TEXT,
                pull_commit TEXT,
                title TEXT,
                url TEXT
            );
        "###));
        Ok(result)
    }
    fn conn(&self) -> Result<Connection, Box<Error + Send + Sync>> {
        Ok(try!(Connection::connect(self.params.clone(), TlsMode::None)))
    }
}

impl Db for PostgresDb {
    fn transaction<T: db::Transaction>(
        &mut self,
        t: T,
    ) -> Result<T::Return, Box<Error + Send + Sync>> {
        let conn = try!(self.conn());
        let mut transaction = PostgresTransaction::new(
            try!(conn.transaction())
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
        let conn = try!(self.conn());
        let result = PostgresTransaction::new(
            try!(conn.transaction())
        ).push_queue(pipeline_id, queue_entry);
        result
    }
    fn pop_queue(
        &mut self,
        pipeline_id: PipelineId,
    ) -> Result<Option<QueueEntry>, Box<Error + Send + Sync>> {
        let conn = try!(self.conn());
        let result = PostgresTransaction::new(
            try!(conn.transaction())
        ).pop_queue(pipeline_id);
        result
    }
    fn list_queue(
        &mut self,
        pipeline_id: PipelineId,
    ) -> Result<Vec<QueueEntry>, Box<Error + Send + Sync>> {
        let conn = try!(self.conn());
        let result = PostgresTransaction::new(
            try!(conn.transaction())
        ).list_queue(pipeline_id);
        result
    }
    fn put_running(
        &mut self,
        pipeline_id: PipelineId,
        running_entry: RunningEntry,
    ) -> Result<(), Box<Error + Send + Sync>> {
        let conn = try!(self.conn());
        let result = PostgresTransaction::new(
            try!(conn.transaction())
        ).put_running(pipeline_id, running_entry);
        result
    }
    fn take_running(
        &mut self,
        pipeline_id: PipelineId,
    ) -> Result<Option<RunningEntry>, Box<Error + Send + Sync>> {
        let conn = try!(self.conn());
        let result = PostgresTransaction::new(
            try!(conn.transaction())
        ).take_running(pipeline_id);
        result
    }
    fn peek_running(
        &mut self,
        pipeline_id: PipelineId,
    ) -> Result<Option<RunningEntry>, Box<Error + Send + Sync>> {
        let conn = try!(self.conn());
        let result = PostgresTransaction::new(
            try!(conn.transaction())
        ).peek_running(pipeline_id);
        result
    }
    fn add_pending(
        &mut self,
        pipeline_id: PipelineId,
        entry: PendingEntry,
    ) -> Result<(), Box<Error + Send + Sync>> {
        let conn = try!(self.conn());
        let result = PostgresTransaction::new(
            try!(conn.transaction())
        ).add_pending(pipeline_id, entry);
        result
    }
    fn take_pending_by_pr(
        &mut self,
        pipeline_id: PipelineId,
        pr: &Pr,
    ) -> Result<Option<PendingEntry>, Box<Error + Send + Sync>> {
        let conn = try!(self.conn());
        let result = PostgresTransaction::new(
            try!(conn.transaction())
        ).take_pending_by_pr(pipeline_id, pr);
        result
    }
    fn peek_pending_by_pr(
        &mut self,
        pipeline_id: PipelineId,
        pr: &Pr,
    ) -> Result<Option<PendingEntry>, Box<Error + Send + Sync>> {
        let conn = try!(self.conn());
        let result = PostgresTransaction::new(
            try!(conn.transaction())
        ).peek_pending_by_pr(pipeline_id, pr);
        result
    }
    fn list_pending(
        &mut self,
        pipeline_id: PipelineId,
    ) -> Result<Vec<PendingEntry>, Box<Error + Send + Sync>> {
        let conn = try!(self.conn());
        let result = PostgresTransaction::new(
            try!(conn.transaction())
        ).list_pending(pipeline_id);
        result
    }
    fn cancel_by_pr(
        &mut self,
        pipeline_id: PipelineId,
        pr: &Pr,
    ) -> Result<(), Box<Error + Send + Sync>> {
        let conn = try!(self.conn());
        let result = PostgresTransaction::new(
            try!(conn.transaction())
        ).cancel_by_pr(pipeline_id, pr);
        result
    }
    fn cancel_by_pr_different_commit(
        &mut self,
        pipeline_id: PipelineId,
        pr: &Pr,
        commit: &Commit,
    ) -> Result<bool, Box<Error + Send + Sync>> {
        let conn = try!(self.conn());
        let result = PostgresTransaction::new(
            try!(conn.transaction())
        ).cancel_by_pr_different_commit(pipeline_id, pr, commit);
        result
    }
    fn set_ci_state(
        &mut self,
        ci_id: CiId,
        ci_state: CiState,
        commit: &Commit,
    ) -> Result<(), Box<Error + Send + Sync>> {
        let conn = try!(self.conn());
        let result = PostgresTransaction::new(
            try!(conn.transaction())
        ).set_ci_state(ci_id, ci_state, commit);
        result
    }
    fn clear_ci_state(
        &mut self,
        ci_id: CiId,
    ) -> Result<(), Box<Error + Send + Sync>> {
        let conn = try!(self.conn());
        let result = PostgresTransaction::new(
            try!(conn.transaction())
        ).clear_ci_state(ci_id);
        result
    }
    fn get_ci_state(
        &mut self,
        ci_id: CiId,
    ) -> Result<Option<(CiState, Commit)>, Box<Error + Send + Sync>> {
        let conn = try!(self.conn());
        let result = PostgresTransaction::new(
            try!(conn.transaction())
        ).get_ci_state(ci_id);
        result
    }
}


pub struct PostgresTransaction<'a> {
    conn: PgTransaction<'a>,
}

impl<'a> PostgresTransaction<'a> {
    pub fn new(conn: PgTransaction<'a>) -> Self {
        PostgresTransaction {
            conn: conn,
        }
    }
}

impl<'a> Db for PostgresTransaction<'a> {
    fn push_queue(
        &mut self,
        pipeline_id: PipelineId,
        QueueEntry{pr, commit, message}: QueueEntry
    ) -> Result<(), Box<Error + Send + Sync>> {
        let sql = r###"
            INSERT INTO queue (pr, pipeline_id, pull_commit, message)
            VALUES ($1, $2, $3, $4)
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
        let trans = try!(self.conn
            .transaction());
        let sql = r###"
            SELECT id, pr, pull_commit, message
            FROM queue
            WHERE pipeline_id = $1
            ORDER BY id ASC LIMIT 1
        "###;
        let item = {
            let stmt = try!(trans.prepare(sql));
            let rows = try!(stmt.query(&[&pipeline_id.0]));
            let rows = rows.iter();
            let mut rows = rows.map(|row| (
                row.get::<_, i32>(0),
                QueueEntry {
                    pr: Pr::from(row.get::<_, String>(1)),
                    commit: Commit::from(row.get::<_, String>(2)),
                    message: row.get::<_, String>(3),
                },
            ));
            rows.next()
        };
        if let Some((id, _)) = item {
            let sql = r###"
                DELETE FROM queue WHERE id = $1
            "###;
            try!(trans.execute(sql, &[&id]));
        }
        try!(trans.commit());
        Ok(item.map(|item| item.1))
    }
    fn list_queue(
        &mut self,
        pipeline_id: PipelineId,
    ) -> Result<Vec<QueueEntry>, Box<Error + Send + Sync>> {
        let sql = r###"
            SELECT pr, pull_commit, message
            FROM queue
            WHERE pipeline_id = $1
            ORDER BY id ASC
        "###;
        let stmt = try!(self.conn.prepare(&sql));
        let rows = try!(stmt.query(&[&pipeline_id.0]));
        let rows = rows.iter();
        let rows = rows.map(|row| QueueEntry {
            pr: Pr::from(row.get::<_, String>(0)),
            commit: Commit::from(row.get::<_, String>(1)),
            message: row.get::<_, String>(2),
        });
        let rows: Vec<QueueEntry> = rows.collect();
        Ok(rows)
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
            INSERT INTO running
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
                ($1, $2, $3, $4, $5, $6, $7)
            ON CONFLICT (pipeline_id) DO UPDATE SET
                pr = $2,
                pull_commit = $3,
                merge_commit = $4,
                message = $5,
                canceled = $6,
                built = $7
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
        let trans = try!(self.conn.transaction());
        let sql = r###"
            SELECT pr, pull_commit, merge_commit, message, canceled, built
            FROM running
            WHERE pipeline_id = $1
        "###;
        let entry = {
            let stmt = try!(trans.prepare(&sql));
            let rows = try!(stmt.query(&[&pipeline_id.0]));
            let rows = rows.iter();
            let mut rows = rows.map(|row| RunningEntry {
                pr: Pr::from(row.get::<_, String>(0)),
                pull_commit: Commit::from(row.get::<_, String>(1)),
                merge_commit: row.get::<_, Option<String>>(2).map(
                    |v| Commit::from(v)
                ),
                message: row.get(3),
                canceled: row.get(4),
                built: row.get(5),
            });
            rows.next()
        };
        let sql = r###"
            DELETE FROM running WHERE pipeline_id = $1
        "###;
        try!(trans.execute(sql, &[&pipeline_id.0]));
        try!(trans.commit());
        Ok(entry)
    }
    fn peek_running(
        &mut self,
        pipeline_id: PipelineId,
    ) -> Result<Option<RunningEntry>, Box<Error + Send + Sync>> {
        let sql = r###"
            SELECT pr, pull_commit, merge_commit, message, canceled, built
            FROM running
            WHERE pipeline_id = $1
        "###;
        let stmt = try!(self.conn.prepare(&sql));
        let rows = try!(stmt.query(&[&pipeline_id.0]));
        let rows = rows.iter();
        let mut rows = rows.map(|row| RunningEntry {
            pr: Pr::from(row.get::<_, String>(0)),
            pull_commit: Commit::from(row.get::<_, String>(1)),
            merge_commit: row.get::<_, Option<String>>(2).map(Commit::from),
            message: row.get(3),
            canceled: row.get(4),
            built: row.get(5),
        });
        Ok(rows.next())
    }
    fn add_pending(
        &mut self,
        pipeline_id: PipelineId,
        entry: PendingEntry,
    ) -> Result<(), Box<Error + Send + Sync>> {
        let trans = try!(self.conn.transaction());
        let sql = r###"
            DELETE FROM pending WHERE pipeline_id = $1 AND pr = $2
        "###;
        try!(trans.execute(sql, &[
            &pipeline_id.0,
            &entry.pr.as_str(),
        ]));
        let sql = r###"
            INSERT INTO pending (pipeline_id, pr, pull_commit, title, url)
            VALUES ($1, $2, $3, $4, $5)
        "###;
        try!(trans.execute(sql, &[
            &pipeline_id.0,
            &entry.pr.as_str(),
            &entry.commit.as_str(),
            &entry.title,
            &entry.url.as_str(),
        ]));
        try!(trans.commit());
        Ok(())
    }
    fn take_pending_by_pr(
        &mut self,
        pipeline_id: PipelineId,
        pr: &Pr,
    ) -> Result<Option<PendingEntry>, Box<Error + Send + Sync>> {
        let trans = try!(self.conn.transaction());
        let sql = r###"
            SELECT id, pr, pull_commit, title, url
            FROM pending
            WHERE pipeline_id = $1 AND pr = $2
        "###;
        let entry = {
            let stmt = try!(trans.prepare(&sql));
            let rows = try!(stmt.query(&[
                &pipeline_id.0,
                &pr.as_str(),
            ]));
            let rows = rows.iter();
            let mut rows = rows.map(|row| (row.get::<_, i32>(0), PendingEntry {
                pr: Pr::from(row.get::<_, String>(1)),
                commit: Commit::from(row.get::<_, String>(2)),
                title: row.get(3),
                url: Url::parse(&row.get::<_, String>(4)).unwrap(),
            }));
            rows.next()
        };
        if let Some(ref entry) = entry {
            let sql = r###"
                DELETE FROM pending WHERE id = $1
            "###;
            try!(trans.execute(sql, &[&entry.0]));
            try!(trans.commit());
        }
        Ok(entry.map(|entry| entry.1))
    }
    fn peek_pending_by_pr(
        &mut self,
        pipeline_id: PipelineId,
        pr: &Pr,
    ) -> Result<Option<PendingEntry>, Box<Error + Send + Sync>> {
        let sql = r###"
            SELECT pr, pull_commit, title, url
            FROM pending
            WHERE pipeline_id = $1 AND pr = $2
        "###;
        let stmt = try!(self.conn.prepare(&sql));
        let rows = try!(stmt.query(&[
            &pipeline_id.0,
            &pr.as_str(),
        ]));
        let rows = rows.iter();
        let mut rows = rows.map(|row| PendingEntry {
            pr: Pr::from(row.get::<_, String>(0)),
            commit: Commit::from(row.get::<_, String>(1)),
            title: row.get(2),
            url: Url::parse(&row.get::<_, String>(3)).unwrap(),
        });
        Ok(rows.next())
    }
    fn list_pending(
        &mut self,
        pipeline_id: PipelineId,
    ) -> Result<Vec<PendingEntry>, Box<Error + Send + Sync>> {
        let sql = r###"
            SELECT pr, pull_commit, title, url
            FROM pending
            WHERE pipeline_id = $1
        "###;
        let stmt = try!(self.conn.prepare(&sql));
        let rows = try!(stmt.query(&[&pipeline_id.0]));
        let rows = rows.iter();
        let rows = rows.map(|row| PendingEntry {
            pr: Pr::from(row.get::<_, String>(0)),
            commit: Commit::from(row.get::<_, String>(1)),
            title: row.get(2),
            url: Url::parse(&row.get::<_, String>(3)).unwrap(),
        });
        let rows: Vec<PendingEntry> = rows.collect();
        Ok(rows)
    }
    fn cancel_by_pr(
        &mut self,
        pipeline_id: PipelineId,
        pr: &Pr,
    ) -> Result<(), Box<Error + Send + Sync>> {
        let sql = r###"
            UPDATE running
            SET canceled = TRUE
            WHERE pipeline_id = $1 AND pr = $2
        "###;
        try!(self.conn.execute(sql, &[
            &pipeline_id.0,
            &pr.as_str(),
        ]));
        let sql = r###"
            DELETE FROM queue
            WHERE pipeline_id = $1 AND pr = $2
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
            SET canceled = TRUE
            WHERE pipeline_id = $1 AND pr = $2 AND pull_commit <> $3
        "###;
        let affected_rows_running = try!(self.conn.execute(sql, &[
            &pipeline_id.0,
            &pr.as_str(),
            &commit.as_str(),
        ]));
        let sql = r###"
            DELETE FROM queue
            WHERE pipeline_id = $1 AND pr = $2 AND pull_commit <> $3
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
            INSERT INTO ci_state (ci_id, ci_state, ci_commit)
            VALUES ($1, $2, $3)
            ON CONFLICT (ci_id) DO UPDATE SET
                ci_state = $2,
                ci_commit = $3
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
            WHERE ci_id = $1
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
            SELECT 
                ci_state, ci_commit
            FROM ci_state
            WHERE ci_id = $1
        "###;
        let stmt = try!(self.conn.prepare(&sql));
        let rows = try!(stmt.query(&[ &ci_id.0 ]));
        let rows = rows.iter();
        let mut rows = rows.map(|row| (
            CiState::from_i32(row.get::<_, i32>(0)),
            Commit::from(row.get::<_, String>(1)),
        ));
        let value = rows.next();
        Ok(value)
    }
}