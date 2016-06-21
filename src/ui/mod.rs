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
pub enum Message<P: Pr> {
    SendResult(PipelineId, P, Status<P>)
}

#[derive(Clone, Debug)]
pub enum Event<P: Pr> {
    Approved(PipelineId, P, Option<P::C>, String),
    Canceled(PipelineId, P),
    Opened(PipelineId, P, P::C),
    Changed(PipelineId, P, P::C),
    Closed(PipelineId, P),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Status<P: Pr> {
    Invalidated,
    NoCommit,
    Unmergeable(P::C),
    StartingBuild(P::C, P::C),
    Testing(P::C, P::C, Option<Url>),
    Success(P::C, P::C, Option<Url>),
    Failure(P::C, P::C, Option<Url>),
    Unmoveable(P::C, P::C),
    Completed(P::C, P::C),
}

/// A series of reviewable changesets and other messages
pub trait Pr: Clone + Debug + Display + Eq + FromStr + Into<String> +
              PartialEq + Send
{
    type C: Commit;
    fn remote(&self) -> <Self::C as Commit>::Remote;
}

impl<P: Pr> GetPipelineId for Event<P> {
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