// This file is released under the same terms as Rust itself.

pub mod jenkins;

use hyper::Url;
use pipeline::{GetPipelineId, PipelineId};
use vcs::Commit;

#[derive(Clone, Debug)]
pub enum Message<C: Commit> {
    StartBuild(PipelineId, C),
}

#[derive(Clone, Debug)]
pub enum Event<C: Commit> {
    BuildStarted(PipelineId, C, Option<Url>),
    BuildSucceeded(PipelineId, C, Option<Url>),
    BuildFailed(PipelineId, C, Option<Url>),
}

impl<C: Commit + 'static> GetPipelineId for Event<C> {
    fn pipeline_id(&self) -> PipelineId {
        match *self {
            Event::BuildStarted(i, _, _) => i,
            Event::BuildSucceeded(i, _, _) => i,
            Event::BuildFailed(i, _, _) => i,
        }
    }
}