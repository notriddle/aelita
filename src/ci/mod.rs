// This file is released under the same terms as Rust itself.

pub mod jenkins;

use vcs::Commit;
use pipeline::{GetPipelineId, PipelineId};

#[derive(Clone, Debug)]
pub enum Message<C: Commit> {
    StartBuild(PipelineId, C),
}

#[derive(Clone, Debug)]
pub enum Event<C: Commit> {
    BuildSucceeded(PipelineId, C),
    BuildFailed(PipelineId, C),
}

impl<C: Commit + 'static> GetPipelineId for Event<C> {
    fn pipeline_id(&self) -> PipelineId {
        match *self {
            Event::BuildSucceeded(i, _) => i,
            Event::BuildFailed(i, _) => i,
        }
    }
}