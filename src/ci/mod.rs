// This file is released under the same terms as Rust itself.

pub mod github_status;
pub mod jenkins;

use hyper::Url;
use pipeline::{GetPipelineId, PipelineId};
use vcs::Commit;

#[derive(Clone, Debug)]
pub enum Message {
    StartBuild(PipelineId, Commit),
}

#[derive(Clone, Debug)]
pub enum Event {
    BuildStarted(PipelineId, Commit, Option<Url>),
    BuildSucceeded(PipelineId, Commit, Option<Url>),
    BuildFailed(PipelineId, Commit, Option<Url>),
}

impl GetPipelineId for Event {
    fn pipeline_id(&self) -> PipelineId {
        match *self {
            Event::BuildStarted(i, _, _) => i,
            Event::BuildSucceeded(i, _, _) => i,
            Event::BuildFailed(i, _, _) => i,
        }
    }
}