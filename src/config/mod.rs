// This file is released under the same terms as Rust itself.

pub mod toml;
pub mod twelvef;

use db::Db;
use pipeline::{PipelineId, WorkerManager};
use ui::Pr;

pub trait WorkerBuilder {
    type Pr: Pr + 'static;
    fn start(
        self
    ) -> (WorkerManager<Self::Pr>, Box<Db<Self::Pr>>);
}

pub trait PipelineConfig {
    fn workers_by_id(&self, PipelineId) -> (usize, usize, usize);
    fn len(&self) -> usize;
}
