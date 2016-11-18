// This file is released under the same terms as Rust itself.

/*! The state store.
 */

pub mod sqlite;
pub mod postgres;

use ci::CiId;
use db::postgres::PostgresDb;
use db::sqlite::SqliteDb;
use hyper::Url;
use ui::Pr;
use pipeline::PipelineId;
use postgres::params::{ConnectParams, IntoConnectParams};
use std::error::Error;
use std::path::{Path, PathBuf};
use vcs::Commit;

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
    pub fn open(
        &self
    ) -> Result<DbBox, Box<Error + Send + Sync>>
    {
        Ok(match *self {
            Builder::Sqlite(ref p) =>
                DbBox::Sqlite(try!(SqliteDb::open(p))),
            Builder::Postgres(ref c) =>
                DbBox::Postgres(try!(PostgresDb::open(c.clone()))),
        })
    }
}

pub enum DbBox {
    Sqlite(SqliteDb),
    Postgres(PostgresDb),
}

impl Db for DbBox {
    fn transaction<T: Transaction>(
        &mut self,
        t: T,
    ) -> Result<T::Return, Box<Error + Send + Sync>> {
        match *self {
            DbBox::Sqlite(ref mut d) => d.transaction(t),
            DbBox::Postgres(ref mut d) => d.transaction(t),
        }
    }
    fn push_queue(
        &mut self,
        pipeline_id: PipelineId,
        queue_entry: QueueEntry,
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
    ) -> Result<Option<QueueEntry>, Box<Error + Send + Sync>> {
        match *self {
            DbBox::Sqlite(ref mut d) => d.pop_queue(pipeline_id),
            DbBox::Postgres(ref mut d) => d.pop_queue(pipeline_id),
        }
    }
    fn list_queue(
        &mut self,
        pipeline_id: PipelineId,
    ) -> Result<Vec<QueueEntry>, Box<Error + Send + Sync>> {
        match *self {
            DbBox::Sqlite(ref mut d) => d.list_queue(pipeline_id),
            DbBox::Postgres(ref mut d) => d.list_queue(pipeline_id),
        }
    }
    fn put_running(
        &mut self,
        pipeline_id: PipelineId,
        running_entry: RunningEntry,
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
    ) -> Result<Option<RunningEntry>, Box<Error + Send + Sync>> {
        match *self {
            DbBox::Sqlite(ref mut d) => d.take_running(pipeline_id),
            DbBox::Postgres(ref mut d) => d.take_running(pipeline_id),
        }
    }
    fn peek_running(
        &mut self,
        pipeline_id: PipelineId,
    ) -> Result<Option<RunningEntry>, Box<Error + Send + Sync>> {
        match *self {
            DbBox::Sqlite(ref mut d) => d.peek_running(pipeline_id),
            DbBox::Postgres(ref mut d) => d.peek_running(pipeline_id),
        }
    }
    fn add_pending(
        &mut self,
        pipeline_id: PipelineId,
        pending_entry: PendingEntry,
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
        pr: &Pr,
    ) -> Result<Option<PendingEntry>, Box<Error + Send + Sync>> {
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
        pr: &Pr,
    ) -> Result<Option<PendingEntry>, Box<Error + Send + Sync>> {
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
    ) -> Result<Vec<PendingEntry>, Box<Error + Send + Sync>> {
        match *self {
            DbBox::Sqlite(ref mut d) => d.list_pending(pipeline_id),
            DbBox::Postgres(ref mut d) => d.list_pending(pipeline_id),
        }
    }
    fn cancel_by_pr(
        &mut self,
        pipeline_id: PipelineId,
        pr: &Pr,
    ) -> Result<(), Box<Error + Send + Sync>> {
        match *self {
            DbBox::Sqlite(ref mut d) => d.cancel_by_pr(pipeline_id, pr),
            DbBox::Postgres(ref mut d) => d.cancel_by_pr(pipeline_id, pr),
        }
    }
    fn cancel_by_pr_different_commit(
        &mut self,
        pipeline_id: PipelineId,
        pr: &Pr,
        commit: &Commit,
    ) -> Result<bool, Box<Error + Send + Sync>> {
        match *self {
            DbBox::Sqlite(ref mut d) =>
                d.cancel_by_pr_different_commit(pipeline_id, pr, commit),
            DbBox::Postgres(ref mut d) =>
                d.cancel_by_pr_different_commit(pipeline_id, pr, commit),
        }
    }
    fn set_ci_state(
        &mut self,
        ci_id: CiId,
        ci_state: CiState,
        commit: &Commit,
    ) -> Result<(), Box<Error + Send + Sync>> {
        match *self {
            DbBox::Sqlite(ref mut d) =>
                d.set_ci_state(ci_id, ci_state, commit),
            DbBox::Postgres(ref mut d) =>
                d.set_ci_state(ci_id, ci_state, commit),
        }
    }
    fn clear_ci_state(
        &mut self,
        ci_id: CiId,
    ) -> Result<(), Box<Error + Send + Sync>> {
        match *self {
            DbBox::Sqlite(ref mut d) =>
                d.clear_ci_state(ci_id),
            DbBox::Postgres(ref mut d) =>
                d.clear_ci_state(ci_id),
        }
    }
    fn get_ci_state(
        &mut self,
        ci_id: CiId,
    ) -> Result<Option<(CiState, Commit)>, Box<Error + Send + Sync>> {
        match *self {
            DbBox::Sqlite(ref mut d) =>
                d.get_ci_state(ci_id),
            DbBox::Postgres(ref mut d) =>
                d.get_ci_state(ci_id),
        }
    }
}


/// A build queue
pub trait Db: Sized {
    fn transaction<T: Transaction>(
        &mut self,
        t: T,
    ) -> Result<T::Return, Box<Error + Send + Sync>> {
        t.run(self)
    }
    fn push_queue(
        &mut self,
        PipelineId,
        QueueEntry,
    ) -> Result<(), Box<Error + Send + Sync>>;
    fn pop_queue(
        &mut self,
        PipelineId,
    ) -> Result<Option<QueueEntry>, Box<Error + Send + Sync>>;
    fn list_queue(
        &mut self,
        PipelineId,
    ) -> Result<Vec<QueueEntry>, Box<Error + Send + Sync>>;
    fn put_running(
        &mut self,
        PipelineId,
        RunningEntry,
    ) -> Result<(), Box<Error + Send + Sync>>;
    fn take_running(
        &mut self,
        PipelineId,
    ) -> Result<Option<RunningEntry>, Box<Error + Send + Sync>>;
    fn peek_running(
        &mut self,
        PipelineId,
    ) -> Result<Option<RunningEntry>, Box<Error + Send + Sync>>;
    fn add_pending(
        &mut self,
        PipelineId,
        PendingEntry,
    ) -> Result<(), Box<Error + Send + Sync>>;
    fn peek_pending_by_pr(
        &mut self,
        PipelineId,
        &Pr,
    ) -> Result<Option<PendingEntry>, Box<Error + Send + Sync>>;
    fn take_pending_by_pr(
        &mut self,
        PipelineId,
        &Pr,
    ) -> Result<Option<PendingEntry>, Box<Error + Send + Sync>>;
    fn list_pending(
        &mut self,
        PipelineId,
    ) -> Result<Vec<PendingEntry>, Box<Error + Send + Sync>>;
    fn cancel_by_pr(
        &mut self,
        PipelineId,
        &Pr,
    ) -> Result<(), Box<Error + Send + Sync>>;
    /// Cancel all queued and running entries in the given pipeline
    /// with the same PR number and a different commit number.
    /// Returns true if an actual cancel occurred.
    fn cancel_by_pr_different_commit(
        &mut self,
        PipelineId,
        &Pr,
        &Commit,
    ) -> Result<bool, Box<Error + Send + Sync>>;
    /// Set the state of a CI job.
    fn set_ci_state(
        &mut self,
        CiId,
        CiState,
        &Commit,
    ) -> Result<(), Box<Error + Send + Sync>>;
    /// Set the state of a CI job.
    fn clear_ci_state(
        &mut self,
        CiId,
    ) -> Result<(), Box<Error + Send + Sync>>;
    /// Get a count of completed / remaining CI jobs.
    fn get_ci_state(
        &mut self,
        CiId,
    ) -> Result<Option<(CiState, Commit)>, Box<Error + Send + Sync>>;
}

pub trait Transaction {
    type Return;
    fn run<D: Db>(
        self,
        &mut D,
    ) -> Result<Self::Return, Box<Error + Send + Sync>>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum CiState {
    Succeeded = 1,
    Failed = 2,
}

impl CiState {
    pub fn from_i32(this: i32) -> CiState {
        match this {
            1 => CiState::Succeeded,
            2 => CiState::Failed,
            x => panic!("Invalid CI state: {}", x),
        }
    }
}

/// An item not yet in the build queue
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingEntry {
    pub commit: Commit,
    pub pr: Pr,
    pub title: String,
    pub url: Url,
}

/// An item in the build queue
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueueEntry {
    pub commit: Commit,
    pub pr: Pr,
    pub message: String,
}

/// An item in the build queue that is currently running
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunningEntry {
    pub pull_commit: Commit,
    pub merge_commit: Option<Commit>,
    pub pr: Pr,
    pub message: String,
    pub canceled: bool,
    pub built: bool,
}
