// This file is released under the same terms as Rust itself.

pub mod toml;
pub mod twelvef;

use ci::CiId;
use db::DbBox;
use pipeline::{PipelineId, WorkerManager};

pub trait WorkerBuilder {
    fn start(
        self
    ) -> (WorkerManager, DbBox);
}

pub trait PipelinesConfig {
    fn by_pipeline_id(&self, PipelineId) -> PipelineConfig;
    fn by_ci_id(&self, CiId) -> PipelineConfig;
    fn len(&self) -> usize;
}

#[derive(Clone)]
pub struct PipelineConfig {
	pub pipeline_id: PipelineId,
	pub ci: Vec<(CiId, usize)>,
	pub ui: usize,
	pub vcs: usize,
}