// This file is released under the same terms as Rust itself.

/*! The state store.
 */

pub mod sqlite;
pub mod postgres;

use db::postgres::PostgresDb;
use db::sqlite::SqliteDb;
use hyper::Url;
use ui::Pr;
use pipeline::PipelineId;
use postgres::{ConnectParams, IntoConnectParams};
use std::error::Error;
use std::path::{Path, PathBuf};
use std::str::FromStr;

pub enum Builder {
    Sqlite(PathBuf),
    Postgres(ConnectParams),
}

impl Builder {
    pub fn from_str(desc: &str) -> Result<Builder, Box<Error + Send + Sync>> {
        Ok(if desc.starts_with("postgresql:") {
            Builder::Postgres(try!(desc.into_connect_params()))
        } else {
            Builder::Sqlite(Path::new(desc).to_owned())
        })
    }
    pub fn open<P: Pr + 'static>(
        &self
    ) -> Result<DbBox<P>, Box<Error + Send + Sync>>
        where <<P as Pr>::C as FromStr>::Err: Error,
              <P as FromStr>::Err: Error
    {
        Ok(match *self {
            Builder::Sqlite(ref p) =>
                DbBox::Sqlite(try!(SqliteDb::open(p))),
            Builder::Postgres(ref c) =>
                DbBox::Postgres(try!(PostgresDb::open(c.clone()))),
        })
    }
}

pub enum DbBox<P: Pr> {
    Sqlite(SqliteDb<P>),
    Postgres(PostgresDb<P>),
}

impl<P: Pr> Db<P> for DbBox<P>
    where <<P as Pr>::C as FromStr>::Err: Error,
          <P as FromStr>::Err: Error
{
    fn transaction<T: Transaction<P>>(
        &mut self,
        t: T,
    ) -> Result<(), Box<Error + Send + Sync>> {
        match *self {
            DbBox::Sqlite(ref mut d) => d.transaction(t),
            DbBox::Postgres(ref mut d) => d.transaction(t),
        }
    }
    fn push_queue(
        &mut self,
        pipeline_id: PipelineId,
        queue_entry: QueueEntry<P>,
    ) -> Result<(), Box<Error + Send + Sync>> {
        match *self {
            DbBox::Sqlite(ref mut d) =>
                d.push_queue(pipeline_id, queue_entry),
            DbBox::Postgres(ref mut d) =>
                d.push_queue(pipeline_id, queue_entry),
        }
    }
    fn pop_queue(
        &mut self,
        pipeline_id: PipelineId,
    ) -> Result<Option<QueueEntry<P>>, Box<Error + Send + Sync>> {
        match *self {
            DbBox::Sqlite(ref mut d) => d.pop_queue(pipeline_id),
            DbBox::Postgres(ref mut d) => d.pop_queue(pipeline_id),
        }
    }
    fn list_queue(
        &mut self,
        pipeline_id: PipelineId,
    ) -> Result<Vec<QueueEntry<P>>, Box<Error + Send + Sync>> {
        match *self {
            DbBox::Sqlite(ref mut d) => d.list_queue(pipeline_id),
            DbBox::Postgres(ref mut d) => d.list_queue(pipeline_id),
        }
    }
    fn put_running(
        &mut self,
        pipeline_id: PipelineId,
        running_entry: RunningEntry<P>
    ) -> Result<(), Box<Error + Send + Sync>> {
        match *self {
            DbBox::Sqlite(ref mut d) =>
                d.put_running(pipeline_id, running_entry),
            DbBox::Postgres(ref mut d) =>
                d.put_running(pipeline_id, running_entry),
        }
    }
    fn take_running(
        &mut self,
        pipeline_id: PipelineId,
    ) -> Result<Option<RunningEntry<P>>, Box<Error + Send + Sync>> {
        match *self {
            DbBox::Sqlite(ref mut d) => d.take_running(pipeline_id),
            DbBox::Postgres(ref mut d) => d.take_running(pipeline_id),
        }
    }
    fn peek_running(
        &mut self,
        pipeline_id: PipelineId,
    ) -> Result<Option<RunningEntry<P>>, Box<Error + Send + Sync>> {
        match *self {
            DbBox::Sqlite(ref mut d) => d.peek_running(pipeline_id),
            DbBox::Postgres(ref mut d) => d.peek_running(pipeline_id),
        }
    }
    fn add_pending(
        &mut self,
        pipeline_id: PipelineId,
        pending_entry: PendingEntry<P>,
    ) -> Result<(), Box<Error + Send + Sync>> {
        match *self {
            DbBox::Sqlite(ref mut d) =>
                d.add_pending(pipeline_id, pending_entry),
            DbBox::Postgres(ref mut d) =>
                d.add_pending(pipeline_id, pending_entry),
        }
    }
    fn peek_pending_by_pr(
        &mut self,
        pipeline_id: PipelineId,
        pr: &P,
    ) -> Result<Option<PendingEntry<P>>, Box<Error + Send + Sync>> {
        match *self {
            DbBox::Sqlite(ref mut d) =>
                d.peek_pending_by_pr(pipeline_id, pr),
            DbBox::Postgres(ref mut d) =>
                d.peek_pending_by_pr(pipeline_id, pr),
        }
    }
    fn take_pending_by_pr(
        &mut self,
        pipeline_id: PipelineId,
        pr: &P,
    ) -> Result<Option<PendingEntry<P>>, Box<Error + Send + Sync>> {
        match *self {
            DbBox::Sqlite(ref mut d) =>
                d.take_pending_by_pr(pipeline_id, pr),
            DbBox::Postgres(ref mut d) =>
                d.take_pending_by_pr(pipeline_id, pr),
        }
    }
    fn list_pending(
        &mut self,
        pipeline_id: PipelineId,
    ) -> Result<Vec<PendingEntry<P>>, Box<Error + Send + Sync>> {
        match *self {
            DbBox::Sqlite(ref mut d) => d.list_pending(pipeline_id),
            DbBox::Postgres(ref mut d) => d.list_pending(pipeline_id),
        }
    }
    fn cancel_by_pr(
        &mut self,
        pipeline_id: PipelineId,
        pr: &P,
    ) -> Result<(), Box<Error + Send + Sync>> {
        match *self {
            DbBox::Sqlite(ref mut d) => d.cancel_by_pr(pipeline_id, pr),
            DbBox::Postgres(ref mut d) => d.cancel_by_pr(pipeline_id, pr),
        }
    }
    fn cancel_by_pr_different_commit(
        &mut self,
        pipeline_id: PipelineId,
        pr: &P,
        commit: &P::C,
    ) -> Result<bool, Box<Error + Send + Sync>> {
        match *self {
            DbBox::Sqlite(ref mut d) =>
                d.cancel_by_pr_different_commit(pipeline_id, pr, commit),
            DbBox::Postgres(ref mut d) =>
                d.cancel_by_pr_different_commit(pipeline_id, pr, commit),
        }
    }
}


/// A build queue
pub trait Db<P: Pr>: Sized {
    fn transaction<T: Transaction<P>>(
        &mut self,
        t: T,
    ) -> Result<(), Box<Error + Send + Sync>> {
        t.run(self)
    }
    fn push_queue(
        &mut self,
        PipelineId,
        QueueEntry<P>,
    ) -> Result<(), Box<Error + Send + Sync>>;
    fn pop_queue(
        &mut self,
        PipelineId,
    ) -> Result<Option<QueueEntry<P>>, Box<Error + Send + Sync>>;
    fn list_queue(
        &mut self,
        PipelineId,
    ) -> Result<Vec<QueueEntry<P>>, Box<Error + Send + Sync>>;
    fn put_running(
        &mut self,
        PipelineId,
        RunningEntry<P>,
    ) -> Result<(), Box<Error + Send + Sync>>;
    fn take_running(
        &mut self,
        PipelineId,
    ) -> Result<Option<RunningEntry<P>>, Box<Error + Send + Sync>>;
    fn peek_running(
        &mut self,
        PipelineId,
    ) -> Result<Option<RunningEntry<P>>, Box<Error + Send + Sync>>;
    fn add_pending(
        &mut self,
        PipelineId,
        PendingEntry<P>,
    ) -> Result<(), Box<Error + Send + Sync>>;
    fn peek_pending_by_pr(
        &mut self,
        PipelineId,
        &P,
    ) -> Result<Option<PendingEntry<P>>, Box<Error + Send + Sync>>;
    fn take_pending_by_pr(
        &mut self,
        PipelineId,
        &P,
    ) -> Result<Option<PendingEntry<P>>, Box<Error + Send + Sync>>;
    fn list_pending(
        &mut self,
        PipelineId,
    ) -> Result<Vec<PendingEntry<P>>, Box<Error + Send + Sync>>;
    fn cancel_by_pr(
        &mut self,
        PipelineId,
        &P,
    ) -> Result<(), Box<Error + Send + Sync>>;
    /// Cancel all queued and running entries in the given pipeline
    /// with the same PR number and a different commit number.
    /// Returns true if an actual cancel occurred.
    fn cancel_by_pr_different_commit(
        &mut self,
        PipelineId,
        &P,
        &P::C,
    ) -> Result<bool, Box<Error + Send + Sync>>;
}

pub trait Transaction<P: Pr> {
    fn run<D: Db<P>>(self, &mut D) -> Result<(), Box<Error + Send + Sync>>;
}

/// An item not yet in the build queue
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingEntry<P: Pr> {
    pub commit: P::C,
    pub pr: P,
    pub title: String,
    pub url: Url,
}

/// An item in the build queue
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueueEntry<P: Pr> {
    pub commit: P::C,
    pub pr: P,
    pub message: String,
}

/// An item in the build queue that is currently running
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunningEntry<P: Pr> {
    pub pull_commit: P::C,
    pub merge_commit: Option<P::C>,
    pub pr: P,
    pub message: String,
    pub canceled: bool,
    pub built: bool,
}