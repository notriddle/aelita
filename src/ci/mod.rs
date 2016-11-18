// This file is released under the same terms as Rust itself.

pub mod github_status;
pub mod jenkins;

use config::PipelinesConfig;
use hyper::Url;
use pipeline::{GetPipelineId, PipelineId};
use vcs::Commit;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct CiId(pub i32);

#[derive(Clone, Debug)]
pub enum Message {
    StartBuild(CiId, Commit),
}

#[derive(Clone, Debug)]
pub enum Event {
    BuildStarted(CiId, Commit, Option<Url>),
    BuildSucceeded(CiId, Commit, Option<Url>),
    BuildFailed(CiId, Commit, Option<Url>),
}

impl GetPipelineId for Event {
    fn pipeline_id<C: PipelinesConfig + ?Sized>(&self, config: &C) -> PipelineId {
        let ci_id = match *self {
            Event::BuildStarted(i, _, _) => i,
            Event::BuildSucceeded(i, _, _) => i,
            Event::BuildFailed(i, _, _) => i,
        };
        config.by_ci_id(ci_id).pipeline_id
    }
}