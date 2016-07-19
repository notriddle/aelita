// This file is released under the same terms as Rust itself.

pub mod git;
pub mod github;

use pipeline::{GetPipelineId, PipelineId};
use std::cmp::Eq;
use std::fmt::{Debug, Display};
use std::str::FromStr;

#[derive(Clone, Debug)]
pub enum Message<C: Commit> {
    MergeToStaging(PipelineId, C, String, C::Remote),
    MoveStagingToMaster(PipelineId, C),
}

#[derive(Clone, Debug)]
pub enum Event<C: Commit> {
    MergedToStaging(PipelineId, C, C),
    FailedMergeToStaging(PipelineId, C),
    MovedToMaster(PipelineId, C),
    FailedMoveToMaster(PipelineId, C),
}

/// A reviewable changeset
pub trait Commit: Clone + Debug + Display + Eq + FromStr + Into<String> +
                  PartialEq + Send {
    type Remote: Clone + Debug + Display + Eq + FromStr + Into<String> +
                 PartialEq + Send;
}

impl<C: Commit + 'static> GetPipelineId for Event<C> {
    fn pipeline_id(&self) -> PipelineId {
        match *self {
        	Event::MergedToStaging(i, _, _) => i,
    		Event::FailedMergeToStaging(i, _) => i,
    		Event::MovedToMaster(i, _) => i,
    		Event::FailedMoveToMaster(i, _) => i,
        }
    }
}
