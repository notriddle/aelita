// This file is released under the same terms as Rust itself.

/*! The state store.
 */

pub mod sqlite;

use ui::Pr;
use pipeline::PipelineId;
use vcs::Commit;

/// A build queue
pub trait Db<C: Commit, P: Pr> {
    fn push_queue(&mut self, id: PipelineId, entry: QueueEntry<C, P>);
    fn pop_queue(&mut self, id: PipelineId) -> Option<QueueEntry<C, P>>;
    fn put_running(&mut self, id: PipelineId, entry: RunningEntry<C, P>);
    fn take_running(&mut self, id: PipelineId) -> Option<RunningEntry<C, P>>;
    fn peek_running(&mut self, id: PipelineId) -> Option<RunningEntry<C, P>>;
    fn cancel_by_pr(&mut self, id: PipelineId, pr: &P);
}

/// An item in the build queue
#[derive(Clone, PartialEq, Eq)]
pub struct QueueEntry<C: Commit, P: Pr> {
    pub commit: C,
    pub pr: P,
    pub message: String,
}

/// An item in the build queue that is currently running
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunningEntry<C: Commit, P: Pr> {
    pub pull_commit: C,
    pub merge_commit: Option<C>,
    pub pr: P,
    pub message: String,
    pub canceled: bool,
}