// This file is released under the same terms as Rust itself.

/*! The state store.
 */

pub mod sqlite;
pub mod postgres;

use hyper::Url;
use ui::Pr;
use pipeline::PipelineId;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use postgres::{ConnectParams, IntoConnectParams};

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
    ) -> Result<Box<Db<P> + Send>, Box<Error + Send + Sync>>
        where <<P as Pr>::C as FromStr>::Err: Error,
              <P as FromStr>::Err: Error
    {
        Ok(match *self {
            Builder::Sqlite(ref p) => Box::new(try!(sqlite::SqliteDb::open(p))),
            Builder::Postgres(ref c) => Box::new(try!(postgres::PostgresDb::open(c.clone()))),
        })
    }
}


/// A build queue
pub trait Db<P: Pr> {
    fn push_queue(&mut self, PipelineId, QueueEntry<P>);
    fn pop_queue(&mut self, PipelineId) -> Option<QueueEntry<P>>;
    fn list_queue(&mut self, PipelineId) -> Vec<QueueEntry<P>>;
    fn put_running(&mut self, PipelineId, RunningEntry<P>);
    fn take_running(&mut self, PipelineId) -> Option<RunningEntry<P>>;
    fn peek_running(&mut self, PipelineId) -> Option<RunningEntry<P>>;
    fn add_pending(&mut self, PipelineId, PendingEntry<P>);
    fn peek_pending_by_pr(&mut self, PipelineId, &P)
        -> Option<PendingEntry<P>>;
    fn take_pending_by_pr(&mut self, PipelineId, &P)
        -> Option<PendingEntry<P>>;
    fn list_pending(&mut self, PipelineId) -> Vec<PendingEntry<P>>;
    fn cancel_by_pr(&mut self, PipelineId, &P);
    /// Cancel all queued and running entries in the given pipeline
    /// with the same PR number and a different commit number.
    /// Returns true if an actual cancel occurred.
    fn cancel_by_pr_different_commit(&mut self, PipelineId, &P, &P::C) -> bool;
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