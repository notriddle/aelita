// This file is released under the same terms as Rust itself.

pub mod toml;
pub mod twelvef;

use db::DbBox;
use pipeline::{PipelineId, WorkerManager};

pub trait WorkerBuilder {
    fn start(
        self
    ) -> (WorkerManager, DbBox);
}

pub trait PipelineConfig {
    fn workers_by_id(&self, PipelineId) -> (usize, usize, usize);
    fn len(&self) -> usize;
}
