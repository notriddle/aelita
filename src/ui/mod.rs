// This file is released under the same terms as Rust itself.

/*! The front-end of the build system. This implements the connector to
    tools like GitHub pull requests, as well as "filters" like access
    control.
 */

pub mod github;
mod comments;

use config::PipelinesConfig;
use hyper::Url;
use pipeline::{GetPipelineId, PipelineId};
use std::fmt::{self, Display};
use vcs::{Commit, Remote};

#[derive(Clone, Debug)]
pub enum Message {
    SendResult(PipelineId, Pr, Status)
}

#[derive(Clone, Debug)]
pub enum Event {
    Approved(PipelineId, Pr, Option<Commit>, String),
    Canceled(PipelineId, Pr),
    Opened(PipelineId, Pr, Commit, String, Url),
    Changed(PipelineId, Pr, Commit, String, Url),
    Closed(PipelineId, Pr),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Status {
    Approved(Commit),
    Invalidated,
    NoCommit,
    Unmergeable(Commit),
    StartingBuild(Commit, Commit),
    Testing(Commit, Commit, Option<Url>),
    Success(Commit, Commit, Option<Url>),
    Failure(Commit, Commit, Option<Url>),
    Unmoveable(Commit, Commit),
    Completed(Commit, Commit),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Pr(String);

impl Display for Pr {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        Display::fmt(&self.0, f)
    }
}

impl From<String> for Pr {
    fn from(s: String) -> Pr {
        Pr(s)
    }
}

impl Into<String> for Pr {
    fn into(self) -> String {
        self.0
    }
}

impl Pr {
    pub fn remote(&self) -> Remote {
        Remote::from(format!("pull/{}/head", self.0))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl GetPipelineId for Event {
    fn pipeline_id<C: PipelinesConfig + ?Sized>(&self, _: &C) -> PipelineId {
        match *self {
            Event::Approved(i, _, _, _) => i,
            Event::Canceled(i, _) => i,
            Event::Opened(i, _, _, _, _) => i,
            Event::Changed(i, _, _, _, _) => i,
            Event::Closed(i, _) => i,
        }
    }
}
