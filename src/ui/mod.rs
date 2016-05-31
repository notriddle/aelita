// This file is released under the same terms as Rust itself.

/*! The front-end of the build system. This implements the connector to
    tools like GitHub pull requests, as well as "filters" like access
    control.
 */

pub mod github;
mod comments;

use hyper::Url;
use pipeline::{GetPipelineId, PipelineId};
use std::cmp::Eq;
use std::fmt::{Debug, Display};
use std::str::FromStr;
use vcs::Commit;

#[derive(Clone, Debug)]
pub enum Message<C: Commit, P: Pr> {
    SendResult(PipelineId, P, Status<C>)
}

#[derive(Clone, Debug)]
pub enum Event<C: Commit, P: Pr> {
    Approved(PipelineId, P, Option<C>, String),
    Canceled(PipelineId, P),
    Opened(PipelineId, P, C),
    Changed(PipelineId, P, C),
    Closed(PipelineId, P),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Status<C: Commit> {
    Invalidated,
    NoCommit,
    Unmergeable(C),
    StartingBuild(C, C),
    Testing(C, C, Option<Url>),
    Success(C, C, Option<Url>),
    Failure(C, C, Option<Url>),
    Unmoveable(C, C),
    Completed(C, C),
}

/// A series of reviewable changesets and other messages
pub trait Pr: Clone + Debug + Display + Eq + FromStr + Into<String> +
              PartialEq + Send {
    fn remote(&self) -> String;
}

impl<C: Commit + 'static, P: Pr + 'static> GetPipelineId for Event<C, P> {
    fn pipeline_id(&self) -> PipelineId {
        match *self {
            Event::Approved(i, _, _, _) => i,
            Event::Canceled(i, _) => i,
            Event::Opened(i, _, _) => i,
            Event::Changed(i, _, _) => i,
            Event::Closed(i, _) => i,
        }
    }
}