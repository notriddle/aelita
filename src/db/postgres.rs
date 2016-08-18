// This file is released under the same terms as Rust itself.

use db::{self, Db, PendingEntry, QueueEntry, RunningEntry};
use hyper::Url;
use postgres::{self, Connection, IntoConnectParams, SslMode};
use std::error::Error;
use std::marker::PhantomData;
use std::str::FromStr;
use ui::Pr;
use pipeline::PipelineId;

pub struct PostgresDb<P>
    where P: Pr + Into<String> + FromStr
{
    _pr: PhantomData<P>,
    conn: Connection,
}

impl<P> PostgresDb<P>
    where P: Pr + Into<String> + FromStr
{
    pub fn open<Q: IntoConnectParams>(
        params: Q
    ) -> Result<Self, Box<Error + Send + Sync>> {
        let conn = try!(Connection::connect(params, SslMode::None));
        try!(conn.batch_execute(r###"
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
        Ok(PostgresDb{
            conn: conn,
            _pr: PhantomData,
        })
    }
}

impl<P> Db<P> for PostgresDb<P>
    where P: Pr + Into<String> + FromStr,
          <P::C as FromStr>::Err: ::std::error::Error,
          <P as FromStr>::Err: ::std::error::Error,
{
    fn transaction<T: db::Transaction<P>>(&mut self, t: T) {
        let mut transaction = PostgresTransaction::new(
            self.conn.transaction().expect("to open transaction")
        );
        let result = t.run(&mut transaction);
        if result {
            transaction.conn.commit().expect("to commit successfully");
        }
    }
    fn push_queue(
        &mut self,
        pipeline_id: PipelineId,
        queue_entry: QueueEntry<P>
    ) {
        PostgresTransaction::new(
            self.conn.transaction().expect("to open transaction")
        ).push_queue(pipeline_id, queue_entry)
    }
    fn pop_queue(
        &mut self,
        pipeline_id: PipelineId,
    ) -> Option<QueueEntry<P>> {
        PostgresTransaction::new(
            self.conn.transaction().expect("to open transaction")
        ).pop_queue(pipeline_id)
    }
    fn list_queue(
        &mut self,
        pipeline_id: PipelineId,
    ) -> Vec<QueueEntry<P>> {
        PostgresTransaction::new(
            self.conn.transaction().expect("to open transaction")
        ).list_queue(pipeline_id)
    }
    fn put_running(
        &mut self,
        pipeline_id: PipelineId,
        running_entry: RunningEntry<P>
    ) {
        PostgresTransaction::new(
            self.conn.transaction().expect("to open transaction")
        ).put_running(pipeline_id, running_entry)
    }
    fn take_running(
        &mut self,
        pipeline_id: PipelineId,
    ) -> Option<RunningEntry<P>> {
        PostgresTransaction::new(
            self.conn.transaction().expect("to open transaction")
        ).take_running(pipeline_id)
    }
    fn peek_running(
        &mut self,
        pipeline_id: PipelineId,
    ) -> Option<RunningEntry<P>> {
        PostgresTransaction::new(
            self.conn.transaction().expect("to open transaction")
        ).peek_running(pipeline_id)
    }
    fn add_pending(
        &mut self,
        pipeline_id: PipelineId,
        entry: PendingEntry<P>,
    ) {
        PostgresTransaction::new(
            self.conn.transaction().expect("to open transaction")
        ).add_pending(pipeline_id, entry)
    }
    fn take_pending_by_pr(
        &mut self,
        pipeline_id: PipelineId,
        pr: &P,
    ) -> Option<PendingEntry<P>> {
        PostgresTransaction::new(
            self.conn.transaction().expect("to open transaction")
        ).take_pending_by_pr(pipeline_id, pr)
    }
    fn peek_pending_by_pr(
        &mut self,
        pipeline_id: PipelineId,
        pr: &P,
    ) -> Option<PendingEntry<P>> {
        PostgresTransaction::new(
            self.conn.transaction().expect("to open transaction")
        ).peek_pending_by_pr(pipeline_id, pr)
    }
    fn list_pending(
        &mut self,
        pipeline_id: PipelineId,
    ) -> Vec<PendingEntry<P>> {
        PostgresTransaction::new(
            self.conn.transaction().expect("to open transaction")
        ).list_pending(pipeline_id)
    }
    fn cancel_by_pr(&mut self, pipeline_id: PipelineId, pr: &P) {
        PostgresTransaction::new(
            self.conn.transaction().expect("to open transaction")
        ).cancel_by_pr(pipeline_id, pr)
    }
    fn cancel_by_pr_different_commit(
        &mut self,
        pipeline_id: PipelineId,
        pr: &P,
        commit: &P::C,
    ) -> bool {
        PostgresTransaction::new(
            self.conn.transaction().expect("to open transaction")
        ).cancel_by_pr_different_commit(pipeline_id, pr, commit)
    }
}


pub struct PostgresTransaction<'a, P>
    where P: Pr + Into<String> + FromStr
{
    _pr: PhantomData<P>,
    conn: postgres::Transaction<'a>,
}

impl<'a, P> PostgresTransaction<'a, P>
    where P: Pr + Into<String> + FromStr
{
    pub fn new(conn: postgres::Transaction<'a>) -> Self {
        PostgresTransaction {
            _pr: PhantomData,
            conn: conn,
        }
    }
}

impl<'a, P> Db<P> for PostgresTransaction<'a, P>
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
            VALUES ($1, $2, $3, $4)
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
        let trans = self.conn
            .transaction()
            .expect("Start pop-from-queue transaction");
        let sql = r###"
            SELECT id, pr, pull_commit, message
            FROM queue
            WHERE pipeline_id = $1
            ORDER BY id ASC LIMIT 1
        "###;
        let item = {
            let stmt = trans.prepare(sql).expect("Pop from queue");
            let rows = stmt.query(&[&pipeline_id.0]).expect("pop from queue");
            let rows = rows.iter();
            let mut rows = rows.map(|row| (
                row.get::<_, i32>(0),
                QueueEntry {
                    pr: P::from_str(&row.get::<_, String>(1)).unwrap(),
                    commit: P::C::from_str(&row.get::<_, String>(2)).unwrap(),
                    message: row.get::<_, String>(3),
                },
            ));
            rows.next()
        };
        if let Some((id, _)) = item {
            let sql = r###"
                DELETE FROM queue WHERE id = $1
            "###;
            trans.execute(sql, &[&id]).expect("Delete pop-from-queue row");
        }
        trans.commit().expect("Commit pop-from-queue transaction");
        item.map(|item| item.1)
    }
    fn list_queue(
        &mut self,
        pipeline_id: PipelineId,
    ) -> Vec<QueueEntry<P>> {
        let sql = r###"
            SELECT pr, pull_commit, message
            FROM queue
            WHERE pipeline_id = $1
            ORDER BY id ASC
        "###;
        let stmt = self.conn.prepare(&sql)
            .expect("Prepare list-queue query");
        let rows = stmt.query(&[&pipeline_id.0])
            .expect("Get queue entry");
        let rows = rows.iter();
        let rows = rows.map(|row| QueueEntry {
            pr: P::from_str(&row.get::<_, String>(0)).unwrap(),
            commit: P::C::from_str(&row.get::<_, String>(1)).unwrap(),
            message: row.get::<_, String>(2),
        });
        let rows: Vec<QueueEntry<P>> = rows.collect();
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
        let trans = self.conn.transaction()
            .expect("Start take-running transaction");
        let sql = r###"
            SELECT pr, pull_commit, merge_commit, message, canceled, built
            FROM running
            WHERE pipeline_id = $1
        "###;
        let entry = {
            let stmt = trans.prepare(&sql)
                .expect("Prepare take-running query");
            let rows = stmt.query(&[&pipeline_id.0])
                .expect("To take running");
            let rows = rows.iter();
            let mut rows = rows.map(|row| RunningEntry {
                pr: P::from_str(&row.get::<_, String>(0)[..]).unwrap(),
                pull_commit: P::C::from_str(&row.get::<_, String>(1)[..])
                    .unwrap(),
                merge_commit: row.get::<_, Option<String>>(2).map(
                    |v| P::C::from_str(&v).unwrap()
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
        trans.execute(sql, &[&pipeline_id.0]).expect("Remove running entry");
        trans.commit().expect("Commit take-running transaction");
        entry
    }
    fn peek_running(
        &mut self,
        pipeline_id: PipelineId,
    ) -> Option<RunningEntry<P>> {
        let sql = r###"
            SELECT pr, pull_commit, merge_commit, message, canceled, built
            FROM running
            WHERE pipeline_id = $1
        "###;
        let stmt = self.conn.prepare(&sql)
            .expect("Prepare peek-running query");
        let rows = stmt.query(&[&pipeline_id.0]);
        let rows = rows.expect("Get running query");
        let rows = rows.iter();
        let mut rows = rows.map(|row| RunningEntry {
            pr: P::from_str(&row.get::<_, String>(0)[..]).unwrap(),
            pull_commit: P::C::from_str(&row.get::<_, String>(1)[..])
                .unwrap(),
            merge_commit: row.get::<_, Option<String>>(2).map(
                |v| P::C::from_str(&v).unwrap()
            ),
            message: row.get(3),
            canceled: row.get(4),
            built: row.get(5),
        });
        rows.next()
    }
    fn add_pending(
        &mut self,
        pipeline_id: PipelineId,
        entry: PendingEntry<P>,
    ) {
        let trans = self.conn.transaction()
            .expect("Start add-pending transaction");
        let sql = r###"
            DELETE FROM pending WHERE pipeline_id = $1 AND pr = $2
        "###;
        trans.execute(sql, &[
            &pipeline_id.0,
            &<P as Into<String>>::into(entry.pr.clone()),
        ]).expect("Remove pending entry");
        let sql = r###"
            INSERT INTO pending (pipeline_id, pr, pull_commit, title, url)
            VALUES ($1, $2, $3, $4, $5)
        "###;
        trans.execute(sql, &[
            &pipeline_id.0,
            &<P as Into<String>>::into(entry.pr.clone()),
            &<P::C as Into<String>>::into(entry.commit.clone()),
            &entry.title,
            &entry.url.as_str(),
        ]).expect("Add pending entry");
        trans.commit().expect("Commit add-pending transaction");
    }
    fn take_pending_by_pr(
        &mut self,
        pipeline_id: PipelineId,
        pr: &P,
    ) -> Option<PendingEntry<P>> {
        let trans = self.conn.transaction()
            .expect("Start take-pending transaction");
        let sql = r###"
            SELECT id, pr, pull_commit, title, url
            FROM pending
            WHERE pipeline_id = $1 AND pr = $2
        "###;
        let entry = {
            let stmt = trans.prepare(&sql)
                .expect("Prepare take-pending query");
            let rows = stmt.query(&[
                &pipeline_id.0,
                &<P as Into<String>>::into(pr.clone()),
            ]);
            let rows = rows.expect("Get pending entry");
            let rows = rows.iter();
            let mut rows = rows.map(|row| (row.get::<_, i32>(0), PendingEntry {
                pr: P::from_str(&row.get::<_, String>(1)[..]).unwrap(),
                commit: P::C::from_str(&row.get::<_, String>(2)[..])
                    .unwrap(),
                title: row.get(3),
                url: Url::parse(&row.get::<_, String>(4)).unwrap(),
            }));
            rows.next()
        };
        if let Some(ref entry) = entry {
            let sql = r###"
                DELETE FROM pending WHERE id = $1
            "###;
            trans.execute(sql, &[&entry.0]).expect("Remove pending entry");
            trans.commit().expect("Commit take-pending transaction");
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
            WHERE pipeline_id = $1 AND pr = $2
        "###;
        let stmt = self.conn.prepare(&sql)
            .expect("Prepare peek-pending query");
        let rows = stmt.query(&[
            &pipeline_id.0,
            &<P as Into<String>>::into(pr.clone()),
        ]);
        let rows = rows.expect("Get pending entry");
        let rows = rows.iter();
        let mut rows = rows.map(|row| PendingEntry {
            pr: P::from_str(&row.get::<_, String>(0)[..]).unwrap(),
            commit: P::C::from_str(&row.get::<_, String>(1)[..])
                .unwrap(),
            title: row.get(2),
            url: Url::parse(&row.get::<_, String>(3)).unwrap(),
        });
        rows.next()
    }
    fn list_pending(
        &mut self,
        pipeline_id: PipelineId,
    ) -> Vec<PendingEntry<P>> {
        let sql = r###"
            SELECT pr, pull_commit, title, url
            FROM pending
            WHERE pipeline_id = $1
        "###;
        let stmt = self.conn.prepare(&sql)
            .expect("Prepare peek-pending query");
        let rows = stmt.query(&[&pipeline_id.0]);
        let rows = rows.expect("Get pending entry");
        let rows = rows.iter();
        let rows = rows.map(|row| PendingEntry {
            pr: P::from_str(&row.get::<_, String>(0)[..]).unwrap(),
            commit: P::C::from_str(&row.get::<_, String>(1)[..])
                .unwrap(),
            title: row.get(2),
            url: Url::parse(&row.get::<_, String>(3)).unwrap(),
        });
        let rows: Vec<PendingEntry<P>> = rows.collect();
        rows
    }
    fn cancel_by_pr(&mut self, pipeline_id: PipelineId, pr: &P) {
        let sql = r###"
            UPDATE running
            SET canceled = TRUE
            WHERE pipeline_id = $1 AND pr = $2
        "###;
        self.conn.execute(sql, &[
            &pipeline_id.0,
            &<P as Into<String>>::into(pr.clone()),
        ]).expect("Cancel running PR");
        let sql = r###"
            DELETE FROM queue
            WHERE pipeline_id = $1 AND pr = $2
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
            SET canceled = TRUE
            WHERE pipeline_id = $1 AND pr = $2 AND pull_commit <> $3
        "###;
        let affected_rows_running = self.conn.execute(sql, &[
            &pipeline_id.0,
            &<P as Into<String>>::into(pr.clone()),
            &<P::C as Into<String>>::into(commit.clone()),
        ]).expect("Cancel running PR");
        let sql = r###"
            DELETE FROM queue
            WHERE pipeline_id = $1 AND pr = $2 AND pull_commit <> $3
        "###;
        let affected_rows_queue = self.conn.execute(sql, &[
            &pipeline_id.0,
            &<P as Into<String>>::into(pr.clone()),
            &<P::C as Into<String>>::into(commit.clone()),
        ]).expect("Cancel queue entries");
        affected_rows_queue != 0 || affected_rows_running != 0
    }
}