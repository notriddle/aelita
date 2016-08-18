// This file is released under the same terms as Rust itself.

use db::{self, Db, PendingEntry, QueueEntry, RunningEntry};
use hyper::Url;
use rusqlite::{self, Connection};
use std::convert::AsRef;
use std::marker::PhantomData;
use std::path::Path;
use std::str::FromStr;
use ui::Pr;
use pipeline::PipelineId;

pub struct SqliteDb<P>
    where P: Pr + Into<String> + FromStr
{
    _pr: PhantomData<P>,
    conn: Connection,
}

impl<P> SqliteDb<P>
    where P: Pr + Into<String> + FromStr
{
    pub fn open<Q: AsRef<Path>>(path: Q) -> rusqlite::Result<Self> {
        let conn = try!(Connection::open(path));
        try!(conn.execute_batch(r###"
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
            _pr: PhantomData,
        })
    }
}


impl<P> Db<P> for SqliteDb<P>
    where P: Pr + Into<String> + FromStr,
          <P::C as FromStr>::Err: ::std::error::Error,
          <P as FromStr>::Err: ::std::error::Error,
{
    fn transaction<T: db::Transaction<P>>(&mut self, t: T) {
        let mut transaction = SqliteTransaction::new(
            self.conn.transaction().expect("to open transaction")
        );
        let result = t.run(&mut transaction);
        if result {
            transaction.conn.commit().expect("to be able to commit");
        }
    }
    fn push_queue(
        &mut self,
        pipeline_id: PipelineId,
        queue_entry: QueueEntry<P>
    ) {
        SqliteTransaction::new(
            self.conn.transaction().expect("to open transaction")
        ).push_queue(pipeline_id, queue_entry)
    }
    fn pop_queue(
        &mut self,
        pipeline_id: PipelineId,
    ) -> Option<QueueEntry<P>> {
        SqliteTransaction::new(
            self.conn.transaction().expect("to open transaction")
        ).pop_queue(pipeline_id)
    }
    fn list_queue(
        &mut self,
        pipeline_id: PipelineId,
    ) -> Vec<QueueEntry<P>> {
        SqliteTransaction::new(
            self.conn.transaction().expect("to open transaction")
        ).list_queue(pipeline_id)
    }
    fn put_running(
        &mut self,
        pipeline_id: PipelineId,
        running_entry: RunningEntry<P>
    ) {
        SqliteTransaction::new(
            self.conn.transaction().expect("to open transaction")
        ).put_running(pipeline_id, running_entry)
    }
    fn take_running(
        &mut self,
        pipeline_id: PipelineId,
    ) -> Option<RunningEntry<P>> {
        SqliteTransaction::new(
            self.conn.transaction().expect("to open transaction")
        ).take_running(pipeline_id)
    }
    fn peek_running(
        &mut self,
        pipeline_id: PipelineId,
    ) -> Option<RunningEntry<P>> {
        SqliteTransaction::new(
            self.conn.transaction().expect("to open transaction")
        ).peek_running(pipeline_id)
    }
    fn add_pending(
        &mut self,
        pipeline_id: PipelineId,
        entry: PendingEntry<P>,
    ) {
        SqliteTransaction::new(
            self.conn.transaction().expect("to open transaction")
        ).add_pending(pipeline_id, entry)
    }
    fn take_pending_by_pr(
        &mut self,
        pipeline_id: PipelineId,
        pr: &P,
    ) -> Option<PendingEntry<P>> {
        SqliteTransaction::new(
            self.conn.transaction().expect("to open transaction")
        ).take_pending_by_pr(pipeline_id, pr)
    }
    fn peek_pending_by_pr(
        &mut self,
        pipeline_id: PipelineId,
        pr: &P,
    ) -> Option<PendingEntry<P>> {
        SqliteTransaction::new(
            self.conn.transaction().expect("to open transaction")
        ).peek_pending_by_pr(pipeline_id, pr)
    }
    fn list_pending(
        &mut self,
        pipeline_id: PipelineId,
    ) -> Vec<PendingEntry<P>> {
        SqliteTransaction::new(
            self.conn.transaction().expect("to open transaction")
        ).list_pending(pipeline_id)
    }
    fn cancel_by_pr(&mut self, pipeline_id: PipelineId, pr: &P) {
        SqliteTransaction::new(
            self.conn.transaction().expect("to open transaction")
        ).cancel_by_pr(pipeline_id, pr)
    }
    fn cancel_by_pr_different_commit(
        &mut self,
        pipeline_id: PipelineId,
        pr: &P,
        commit: &P::C,
    ) -> bool {
        SqliteTransaction::new(
            self.conn.transaction().expect("to open transaction")
        ).cancel_by_pr_different_commit(pipeline_id, pr, commit)
    }
}


pub struct SqliteTransaction<'a, P>
    where P: Pr + Into<String> + FromStr
{
    _pr: PhantomData<P>,
    conn: rusqlite::Transaction<'a>,
}

impl<'a, P> SqliteTransaction<'a, P>
    where P: Pr + Into<String> + FromStr
{
    pub fn new(conn: rusqlite::Transaction<'a>) -> Self {
        SqliteTransaction {
            _pr: PhantomData,
            conn: conn,
        }
    }
}

impl<'a, P> Db<P> for SqliteTransaction<'a, P>
    where P: Pr + Into<String> + FromStr,
          <P::C as FromStr>::Err: ::std::error::Error,
          <P as FromStr>::Err: ::std::error::Error,
{
    fn push_queue(
        &mut self,
        pipeline_id: PipelineId,
        QueueEntry{pr, commit, message}: QueueEntry<P>
    ) {
        let sql = r###"
            INSERT INTO queue (pr, pipeline_id, pull_commit, message)
            VALUES (?, ?, ?, ?)
        "###;
        self.conn.execute(sql, &[
            &pr.into(),
            &pipeline_id.0,
            &commit.into(),
            &message,
        ]).expect("Push-to-queue");
    }
    fn pop_queue(
        &mut self,
        pipeline_id: PipelineId,
    ) -> Option<QueueEntry<P>> {
        let sql = r###"
            SELECT id, pr, pull_commit, message
            FROM queue
            WHERE pipeline_id = ?
            ORDER BY id ASC LIMIT 1
        "###;
        let item = {
            let mut stmt = self.conn.prepare(sql).expect("Pop from queue");
            let mut rows = stmt
            .query_map(&[&pipeline_id.0], |row| (
                row.get::<_, i32>(0),
                QueueEntry {
                    pr: P::from_str(&row.get::<_, String>(1)).unwrap(),
                    commit: P::C::from_str(&row.get::<_, String>(2)).unwrap(),
                    message: row.get::<_, String>(3),
                },
            )).expect("Map pop from queue");
            rows.next().map(|item| item.expect("Retrieve pop from queue"))
        };
        if let Some((id, _)) = item {
            let sql = r###"
                DELETE FROM queue WHERE id = ?
            "###;
            self.conn.execute(sql, &[&id]).expect("Delete pop-from-queue row");
        }
        item.map(|item| item.1)
    }
    fn list_queue(
        &mut self,
        pipeline_id: PipelineId,
    ) -> Vec<QueueEntry<P>> {
        let sql = r###"
            SELECT pr, pull_commit, message
            FROM queue
            WHERE pipeline_id = ?
            ORDER BY id ASC
        "###;
        let mut stmt = self.conn.prepare(&sql)
            .expect("Prepare list-queue query");
        let rows = stmt.query_map(&[&pipeline_id.0], |row| QueueEntry {
                pr: P::from_str(&row.get::<_, String>(0)).unwrap(),
                commit: P::C::from_str(&row.get::<_, String>(1)).unwrap(),
                message: row.get::<_, String>(2),
            })
            .expect("Get queue entry");
        let rows: Vec<QueueEntry<P>> = rows.map(|item| {
            item.expect("Retrieve queue entry")
        }).collect();
        rows
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
        }: RunningEntry<P>
    ) {
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
        self.conn.execute(sql, &[
            &pipeline_id.0,
            &<P as Into<String>>::into(pr),
            &<P::C as Into<String>>::into(pull_commit),
            &merge_commit.map(|m| <P::C as Into<String>>::into(m)),
            &message,
            &canceled,
            &built,
        ]).expect("Put running");
    }
    fn take_running(
        &mut self,
        pipeline_id: PipelineId,
    ) -> Option<RunningEntry<P>> {
        let sql = r###"
            SELECT pr, pull_commit, merge_commit, message, canceled, built
            FROM running
            WHERE pipeline_id = ?
        "###;
        let entry = {
            let mut stmt = self.conn.prepare(&sql)
                .expect("Prepare take-running query");
            let mut rows = stmt
                .query_map(&[&pipeline_id.0], |row| RunningEntry {
                    pr: P::from_str(&row.get::<_, String>(0)[..]).unwrap(),
                    pull_commit: P::C::from_str(&row.get::<_, String>(1)[..])
                        .unwrap(),
                    merge_commit: row.get::<_, Option<String>>(2).map(
                        |v| P::C::from_str(&v).unwrap()
                    ),
                    message: row.get(3),
                    canceled: row.get(4),
                    built: row.get(5),
                }).expect("Get running entry");
            rows.next().map(|item| item.expect("Retrieve running entry"))
        };
        let sql = r###"
            DELETE FROM running WHERE pipeline_id = ?
        "###;
        self.conn.execute(sql, &[&pipeline_id.0])
            .expect("Remove running entry");
        entry
    }
    fn peek_running(
        &mut self,
        pipeline_id: PipelineId,
    ) -> Option<RunningEntry<P>> {
        let sql = r###"
            SELECT pr, pull_commit, merge_commit, message, canceled, built
            FROM running
            WHERE pipeline_id = ?
        "###;
        let mut stmt = self.conn.prepare(&sql)
            .expect("Prepare peek-running query");
        let mut rows = stmt
            .query_map(&[&pipeline_id.0], |row| RunningEntry {
                pr: P::from_str(&row.get::<_, String>(0)[..]).unwrap(),
                pull_commit: P::C::from_str(&row.get::<_, String>(1)[..])
                    .unwrap(),
                merge_commit: row.get::<_, Option<String>>(2).map(
                    |v| P::C::from_str(&v).unwrap()
                ),
                message: row.get(3),
                canceled: row.get(4),
                built: row.get(5),
            })
            .expect("Get running entry");
        rows.next()
            .map(|item| item.expect("Retrieve running entry"))
    }
    fn add_pending(
        &mut self,
        pipeline_id: PipelineId,
        entry: PendingEntry<P>,
    ) {
        let sql = r###"
            DELETE FROM pending WHERE pipeline_id = ? AND pr = ?
        "###;
        self.conn.execute(sql, &[
            &pipeline_id.0,
            &<P as Into<String>>::into(entry.pr.clone()),
        ]).expect("Remove pending entry");
        let sql = r###"
            INSERT INTO pending (pipeline_id, pr, pull_commit, title, url)
            VALUES (?, ?, ?, ?, ?)
        "###;
        self.conn.execute(sql, &[
            &pipeline_id.0,
            &<P as Into<String>>::into(entry.pr.clone()),
            &<P::C as Into<String>>::into(entry.commit.clone()),
            &entry.title,
            &entry.url.as_str(),
        ]).expect("Add pending entry");
    }
    fn take_pending_by_pr(
        &mut self,
        pipeline_id: PipelineId,
        pr: &P,
    ) -> Option<PendingEntry<P>> {
        let sql = r###"
            SELECT id, pr, pull_commit, title, url
            FROM pending
            WHERE pipeline_id = ? AND pr = ?
        "###;
        let entry = {
            let mut stmt = self.conn.prepare(&sql)
                .expect("Prepare take-pending query");
            let mut rows = stmt
                .query_map(&[
                    &pipeline_id.0,
                    &<P as Into<String>>::into(pr.clone()),
                ], |row| (row.get::<_, i64>(0), PendingEntry {
                    pr: P::from_str(&row.get::<_, String>(1)[..]).unwrap(),
                    commit: P::C::from_str(&row.get::<_, String>(2)[..])
                        .unwrap(),
                    title: row.get(3),
                    url: Url::parse(&row.get::<_, String>(4)).unwrap(),
                })).expect("Get pending entry");
            rows.next().map(|item| item.expect("Retrieve pending entry"))
        };
        if let Some(ref entry) = entry {
            let sql = r###"
                DELETE FROM pending WHERE id = ?
            "###;
            self.conn.execute(sql, &[&entry.0]).expect("Remove pending entry");
        }
        entry.map(|entry| entry.1)
    }
    fn peek_pending_by_pr(
        &mut self,
        pipeline_id: PipelineId,
        pr: &P,
    ) -> Option<PendingEntry<P>> {
        let sql = r###"
            SELECT pr, pull_commit, title, url
            FROM pending
            WHERE pipeline_id = ? AND pr = ?
        "###;
        let mut stmt = self.conn.prepare(&sql)
            .expect("Prepare peek-pending query");
        let mut rows = stmt
            .query_map(&[
                &pipeline_id.0,
                &<P as Into<String>>::into(pr.clone()),
            ], |row| PendingEntry {
                pr: P::from_str(&row.get::<_, String>(0)[..]).unwrap(),
                commit: P::C::from_str(&row.get::<_, String>(1)[..])
                    .unwrap(),
                title: row.get(2),
                url: Url::parse(&row.get::<_, String>(3)).unwrap(),
            })
            .expect("Get pending entry");
        rows.next()
            .map(|item| item.expect("Retrieve pending entry"))
    }
    fn list_pending(
        &mut self,
        pipeline_id: PipelineId,
    ) -> Vec<PendingEntry<P>> {
        let sql = r###"
            SELECT pr, pull_commit, title, url
            FROM pending
            WHERE pipeline_id = ?
        "###;
        let mut stmt = self.conn.prepare(&sql)
            .expect("Prepare peek-pending query");
        let rows = stmt.query_map(&[&pipeline_id.0], |row| PendingEntry {
                pr: P::from_str(&row.get::<_, String>(0)[..]).unwrap(),
                commit: P::C::from_str(&row.get::<_, String>(1)[..])
                    .unwrap(),
                title: row.get(2),
                url: Url::parse(&row.get::<_, String>(3)).unwrap(),
            })
            .expect("Get pending entry");
        let rows: Vec<PendingEntry<P>> = rows.map(|item| {
            item.expect("Retrieve pending entry")
        }).collect();
        rows
    }
    fn cancel_by_pr(&mut self, pipeline_id: PipelineId, pr: &P) {
        let sql = r###"
            UPDATE running
            SET canceled = 1
            WHERE pipeline_id = ? AND pr = ?
        "###;
        self.conn.execute(sql, &[
            &pipeline_id.0,
            &<P as Into<String>>::into(pr.clone()),
        ]).expect("Cancel running PR");
        let sql = r###"
            DELETE FROM queue
            WHERE pipeline_id = ? AND pr = ?
        "###;
        self.conn.execute(sql, &[
            &pipeline_id.0,
            &<P as Into<String>>::into(pr.clone()),
        ]).expect("Cancel queue entries");
    }
    fn cancel_by_pr_different_commit(
        &mut self,
        pipeline_id: PipelineId,
        pr: &P,
        commit: &P::C,
    ) -> bool {
        let sql = r###"
            UPDATE running
            SET canceled = 1
            WHERE pipeline_id = ? AND pr = ? AND pull_commit <> ?
        "###;
        let affected_rows_running = self.conn.execute(sql, &[
            &pipeline_id.0,
            &<P as Into<String>>::into(pr.clone()),
            &<P::C as Into<String>>::into(commit.clone()),
        ]).expect("Cancel running PR");
        let sql = r###"
            DELETE FROM queue
            WHERE pipeline_id = ? AND pr = ? AND pull_commit <> ?
        "###;
        let affected_rows_queue = self.conn.execute(sql, &[
            &pipeline_id.0,
            &<P as Into<String>>::into(pr.clone()),
            &<P::C as Into<String>>::into(commit.clone()),
        ]).expect("Cancel queue entries");
        affected_rows_queue != 0 || affected_rows_running != 0
    }
}