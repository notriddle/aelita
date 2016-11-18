// This file is released under the same terms as Rust itself.

pub mod git;
pub mod github;

use config::PipelinesConfig;
use pipeline::{GetPipelineId, PipelineId};
use std::fmt::{self, Display};

#[derive(Clone, Debug)]
pub enum Message {
    MergeToStaging(PipelineId, Commit, String, Remote),
    MoveStagingToMaster(PipelineId, Commit),
}

#[derive(Clone, Debug)]
pub enum Event {
    MergedToStaging(PipelineId, Commit, Commit),
    FailedMergeToStaging(PipelineId, Commit),
    MovedToMaster(PipelineId, Commit),
    FailedMoveToMaster(PipelineId, Commit),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Commit(String);

impl Display for Commit {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        Display::fmt(&self.0, f)
    }
}

impl From<String> for Commit {
    fn from(s: String) -> Commit {
        Commit(s)
    }
}

impl Into<String> for Commit {
    fn into(self) -> String {
        self.0
    }
}

impl Commit {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Remote(String);

impl From<String> for Remote {
    fn from(s: String) -> Remote {
        Remote(s)
    }
}

impl Into<String> for Remote {
    fn into(self) -> String {
        self.0
    }
}

impl Display for Remote {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        Display::fmt(&self.0, f)
    }
}

impl GetPipelineId for Event {
    fn pipeline_id<C: PipelinesConfig + ?Sized>(&self, _: &C) -> PipelineId {
        match *self {
        	Event::MergedToStaging(i, _, _) => i,
    		Event::FailedMergeToStaging(i, _) => i,
    		Event::MovedToMaster(i, _) => i,
    		Event::FailedMoveToMaster(i, _) => i,
        }
    }
}
