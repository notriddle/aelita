// This file is released under the same terms as Rust itself.

/*! The state store.
 */

pub mod sqlite;

use ui::Pr;
use pipeline::PipelineId;

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