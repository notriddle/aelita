// This file is released under the same terms as Rust itself.

use db::{self, Db, PendingEntry, QueueEntry, RunningEntry};
use hyper::Url;
use postgres::{self, Connection, ConnectParams, IntoConnectParams, SslMode};
use std::error::Error;
use std::marker::PhantomData;
use std::str::FromStr;
use ui::Pr;
use pipeline::PipelineId;

pub struct PostgresDb<P>
    where P: Pr + Into<String> + FromStr
{
    _pr: PhantomData<P>,
    params: ConnectParams,
}

impl<P> PostgresDb<P>
    where P: Pr + Into<String> + FromStr
{
    pub fn open<Q: IntoConnectParams>(
        params: Q
    ) -> Result<Self, Box<Error + Send + Sync>> {
        let result = PostgresDb{
            _pr: PhantomData,
            params: try!(params.into_connect_params()),
        };
        try!(try!(result.conn()).batch_execute(r###"
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
        Ok(try!(Connection::connect(self.params.clone(), SslMode::None)))
    }
}

impl<P> Db<P> for PostgresDb<P>
    where P: Pr + Into<String> + FromStr,
          <P::C as FromStr>::Err: ::std::error::Error,
          <P as FromStr>::Err: ::std::error::Error,
{
    fn transaction<T: db::Transaction<P>>(
        &mut self,
        t: T,
    ) -> Result<(), Box<Error + Send + Sync>> {
        let conn = try!(self.conn());
        let mut transaction = PostgresTransaction::new(
            try!(conn.transaction())
        );
        try!(t.run(&mut transaction));
        try!(transaction.conn.commit());
        Ok(())
    }
    fn push_queue(
        &mut self,
        pipeline_id: PipelineId,
        queue_entry: QueueEntry<P>,
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
    ) -> Result<Option<QueueEntry<P>>, Box<Error + Send + Sync>> {
        let conn = try!(self.conn());
        let result = PostgresTransaction::new(
            try!(conn.transaction())
        ).pop_queue(pipeline_id);
        result
    }
    fn list_queue(
        &mut self,
        pipeline_id: PipelineId,
    ) -> Result<Vec<QueueEntry<P>>, Box<Error + Send + Sync>> {
        let conn = try!(self.conn());
        let result = PostgresTransaction::new(
            try!(conn.transaction())
        ).list_queue(pipeline_id);
        result
    }
    fn put_running(
        &mut self,
        pipeline_id: PipelineId,
        running_entry: RunningEntry<P>
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
    ) -> Result<Option<RunningEntry<P>>, Box<Error + Send + Sync>> {
        let conn = try!(self.conn());
        let result = PostgresTransaction::new(
            try!(conn.transaction())
        ).take_running(pipeline_id);
        result
    }
    fn peek_running(
        &mut self,
        pipeline_id: PipelineId,
    ) -> Result<Option<RunningEntry<P>>, Box<Error + Send + Sync>> {
        let conn = try!(self.conn());
        let result = PostgresTransaction::new(
            try!(conn.transaction())
        ).peek_running(pipeline_id);
        result
    }
    fn add_pending(
        &mut self,
        pipeline_id: PipelineId,
        entry: PendingEntry<P>,
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
        pr: &P,
    ) -> Result<Option<PendingEntry<P>>, Box<Error + Send + Sync>> {
        let conn = try!(self.conn());
        let result = PostgresTransaction::new(
            try!(conn.transaction())
        ).take_pending_by_pr(pipeline_id, pr);
        result
    }
    fn peek_pending_by_pr(
        &mut self,
        pipeline_id: PipelineId,
        pr: &P,
    ) -> Result<Option<PendingEntry<P>>, Box<Error + Send + Sync>> {
        let conn = try!(self.conn());
        let result = PostgresTransaction::new(
            try!(conn.transaction())
        ).peek_pending_by_pr(pipeline_id, pr);
        result
    }
    fn list_pending(
        &mut self,
        pipeline_id: PipelineId,
    ) -> Result<Vec<PendingEntry<P>>, Box<Error + Send + Sync>> {
        let conn = try!(self.conn());
        let result = PostgresTransaction::new(
            try!(conn.transaction())
        ).list_pending(pipeline_id);
        result
    }
    fn cancel_by_pr(
        &mut self,
        pipeline_id: PipelineId, pr: &P
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
        pr: &P,
        commit: &P::C,
    ) -> Result<bool, Box<Error + Send + Sync>> {
        let conn = try!(self.conn());
        let result = PostgresTransaction::new(
            try!(conn.transaction())
        ).cancel_by_pr_different_commit(pipeline_id, pr, commit);
        result
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
    ) -> Result<(), Box<Error + Send + Sync>> {
        let sql = r###"
            INSERT INTO queue (pr, pipeline_id, pull_commit, message)
            VALUES ($1, $2, $3, $4)
        "###;
        try!(self.conn.execute(sql, &[
            &pr.into(),
            &pipeline_id.0,
            &commit.into(),
            &message,
        ]));
        Ok(())
    }
    fn pop_queue(
        &mut self,
        pipeline_id: PipelineId,
    ) -> Result<Option<QueueEntry<P>>, Box<Error + Send + Sync>> {
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
            try!(trans.execute(sql, &[&id]));
        }
        try!(trans.commit());
        Ok(item.map(|item| item.1))
    }
    fn list_queue(
        &mut self,
        pipeline_id: PipelineId,
    ) -> Result<Vec<QueueEntry<P>>, Box<Error + Send + Sync>> {
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
            pr: P::from_str(&row.get::<_, String>(0)).unwrap(),
            commit: P::C::from_str(&row.get::<_, String>(1)).unwrap(),
            message: row.get::<_, String>(2),
        });
        let rows: Vec<QueueEntry<P>> = rows.collect();
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
        }: RunningEntry<P>
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
            &<P as Into<String>>::into(pr),
            &<P::C as Into<String>>::into(pull_commit),
            &merge_commit.map(|m| <P::C as Into<String>>::into(m)),
            &message,
            &canceled,
            &built,
        ]));
        Ok(())
    }
    fn take_running(
        &mut self,
        pipeline_id: PipelineId,
    ) -> Result<Option<RunningEntry<P>>, Box<Error + Send + Sync>> {
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
        try!(trans.execute(sql, &[&pipeline_id.0]));
        try!(trans.commit());
        Ok(entry)
    }
    fn peek_running(
        &mut self,
        pipeline_id: PipelineId,
    ) -> Result<Option<RunningEntry<P>>, Box<Error + Send + Sync>> {
        let sql = r###"
            SELECT pr, pull_commit, merge_commit, message, canceled, built
            FROM running
            WHERE pipeline_id = $1
        "###;
        let stmt = try!(self.conn.prepare(&sql));
        let rows = try!(stmt.query(&[&pipeline_id.0]));
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
        Ok(rows.next())
    }
    fn add_pending(
        &mut self,
        pipeline_id: PipelineId,
        entry: PendingEntry<P>,
    ) -> Result<(), Box<Error + Send + Sync>> {
        let trans = try!(self.conn.transaction());
        let sql = r###"
            DELETE FROM pending WHERE pipeline_id = $1 AND pr = $2
        "###;
        try!(trans.execute(sql, &[
            &pipeline_id.0,
            &<P as Into<String>>::into(entry.pr.clone()),
        ]));
        let sql = r###"
            INSERT INTO pending (pipeline_id, pr, pull_commit, title, url)
            VALUES ($1, $2, $3, $4, $5)
        "###;
        try!(trans.execute(sql, &[
            &pipeline_id.0,
            &<P as Into<String>>::into(entry.pr.clone()),
            &<P::C as Into<String>>::into(entry.commit.clone()),
            &entry.title,
            &entry.url.as_str(),
        ]));
        try!(trans.commit());
        Ok(())
    }
    fn take_pending_by_pr(
        &mut self,
        pipeline_id: PipelineId,
        pr: &P,
    ) -> Result<Option<PendingEntry<P>>, Box<Error + Send + Sync>> {
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
                &<P as Into<String>>::into(pr.clone()),
            ]));
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
            try!(trans.execute(sql, &[&entry.0]));
            try!(trans.commit());
        }
        Ok(entry.map(|entry| entry.1))
    }
    fn peek_pending_by_pr(
        &mut self,
        pipeline_id: PipelineId,
        pr: &P,
    ) -> Result<Option<PendingEntry<P>>, Box<Error + Send + Sync>> {
        let sql = r###"
            SELECT pr, pull_commit, title, url
            FROM pending
            WHERE pipeline_id = $1 AND pr = $2
        "###;
        let stmt = try!(self.conn.prepare(&sql));
        let rows = try!(stmt.query(&[
            &pipeline_id.0,
            &<P as Into<String>>::into(pr.clone()),
        ]));
        let rows = rows.iter();
        let mut rows = rows.map(|row| PendingEntry {
            pr: P::from_str(&row.get::<_, String>(0)[..]).unwrap(),
            commit: P::C::from_str(&row.get::<_, String>(1)[..])
                .unwrap(),
            title: row.get(2),
            url: Url::parse(&row.get::<_, String>(3)).unwrap(),
        });
        Ok(rows.next())
    }
    fn list_pending(
        &mut self,
        pipeline_id: PipelineId,
    ) -> Result<Vec<PendingEntry<P>>, Box<Error + Send + Sync>> {
        let sql = r###"
            SELECT pr, pull_commit, title, url
            FROM pending
            WHERE pipeline_id = $1
        "###;
        let stmt = try!(self.conn.prepare(&sql));
        let rows = try!(stmt.query(&[&pipeline_id.0]));
        let rows = rows.iter();
        let rows = rows.map(|row| PendingEntry {
            pr: P::from_str(&row.get::<_, String>(0)[..]).unwrap(),
            commit: P::C::from_str(&row.get::<_, String>(1)[..])
                .unwrap(),
            title: row.get(2),
            url: Url::parse(&row.get::<_, String>(3)).unwrap(),
        });
        let rows: Vec<PendingEntry<P>> = rows.collect();
        Ok(rows)
    }
    fn cancel_by_pr(
        &mut self,
        pipeline_id: PipelineId,
        pr: &P,
    ) -> Result<(), Box<Error + Send + Sync>> {
        let sql = r###"
            UPDATE running
            SET canceled = TRUE
            WHERE pipeline_id = $1 AND pr = $2
        "###;
        try!(self.conn.execute(sql, &[
            &pipeline_id.0,
            &<P as Into<String>>::into(pr.clone()),
        ]));
        let sql = r###"
            DELETE FROM queue
            WHERE pipeline_id = $1 AND pr = $2
        "###;
        try!(self.conn.execute(sql, &[
            &pipeline_id.0,
            &<P as Into<String>>::into(pr.clone()),
        ]));
        Ok(())
    }
    fn cancel_by_pr_different_commit(
        &mut self,
        pipeline_id: PipelineId,
        pr: &P,
        commit: &P::C,
    ) -> Result<bool, Box<Error + Send + Sync>> {
        let sql = r###"
            UPDATE running
            SET canceled = TRUE
            WHERE pipeline_id = $1 AND pr = $2 AND pull_commit <> $3
        "###;
        let affected_rows_running = try!(self.conn.execute(sql, &[
            &pipeline_id.0,
            &<P as Into<String>>::into(pr.clone()),
            &<P::C as Into<String>>::into(commit.clone()),
        ]));
        let sql = r###"
            DELETE FROM queue
            WHERE pipeline_id = $1 AND pr = $2 AND pull_commit <> $3
        "###;
        let affected_rows_queue = try!(self.conn.execute(sql, &[
            &pipeline_id.0,
            &<P as Into<String>>::into(pr.clone()),
            &<P::C as Into<String>>::into(commit.clone()),
        ]));
        Ok(affected_rows_queue != 0 || affected_rows_running != 0)
    }
}