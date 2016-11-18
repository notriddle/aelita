// This file is released under the same terms as Rust itself.

use ci::{self, CiId};
use config::{PipelineConfig, PipelinesConfig};
use db::{CiState, Db, PendingEntry, QueueEntry, RunningEntry};
use std::error::Error;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;
use ui::{self, Pr};
use vcs::{self, Commit, Remote};
use view;

pub struct WorkerManager {
    pub cis: Vec<WorkerThread<
        ci::Event,
        ci::Message,
    >>,
    pub uis: Vec<WorkerThread<
        ui::Event,
        ui::Message,
    >>,
    pub vcss: Vec<WorkerThread<
        vcs::Event,
        vcs::Message,
    >>,
    pub view: Option<WorkerThread<
        view::Event,
        view::Message,
    >>,
    pub pipelines: Box<PipelinesConfig>,
}

impl WorkerManager {
    pub fn pipeline_by_id<'a>(
        &'a self,
        pipeline_id: PipelineId,
    ) -> Option<
        Pipeline<
            'a,
            WorkerThread<ci::Event, ci::Message>,
            WorkerThread<ui::Event, ui::Message>,
            WorkerThread<vcs::Event, vcs::Message>,
        >
    > {
        let PipelineConfig{ci, ui, vcs, pipeline_id: _} = self.pipelines.by_pipeline_id(pipeline_id);
        if let (Some(ui), Some(vcs)) = (
            self.uis.get(ui),
            self.vcss.get(vcs)
        ) {
            let ci = ci.iter().flat_map(|&(id, idx)| self.cis.get(idx).map(|ci| (id, ci))).collect();
            Some(Pipeline::new(pipeline_id, ci, ui, vcs))
        } else {
            None
        }
    }
}

pub trait Worker<E: Send + Clone, M: Send + Clone> {
    fn run(&self, recv_msg: Receiver<M>, send_event: Sender<E>);
}

pub struct WorkerThread<E: Send + Clone + 'static, M: Send + Clone + 'static> {
    pub recv_event: Receiver<E>,
    pub send_msg: Sender<M>,
}

impl<E: Send + Clone + 'static, M: Send + Clone + 'static> WorkerThread<E, M> {
    pub fn start<T: Worker<E, M> + Send + 'static>(worker: T) -> Self {
        let (send_msg, recv_msg) = channel();
        let (send_event, recv_event) = channel();
        thread::spawn(move || {
            worker.run(recv_msg, send_event);
        });
        WorkerThread {
            recv_event: recv_event,
            send_msg: send_msg,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PipelineId(pub i32);

pub trait Ci {
    fn start_build(&self, ci_id: CiId, commit: Commit);
}

impl Ci for WorkerThread<ci::Event, ci::Message> {
    fn start_build(&self, ci_id: CiId, commit: Commit) {
        self.send_msg.send(ci::Message::StartBuild(ci_id, commit))
            .unwrap();
    }
}

pub trait Ui {
    fn send_result(&self, PipelineId, Pr, ui::Status);
}

impl Ui for WorkerThread<ui::Event, ui::Message> {
    fn send_result(
        &self,
        pipeline_id: PipelineId,
        pr: Pr,
        status: ui::Status,
    ) {
        self.send_msg.send(ui::Message::SendResult(pipeline_id, pr, status))
            .unwrap();
    }
}

pub trait Vcs {
    fn merge_to_staging(&self, PipelineId, Commit, String, Remote);
    fn move_staging_to_master(&self, PipelineId, Commit);
}

impl Vcs for WorkerThread<vcs::Event, vcs::Message> {
    fn merge_to_staging(
        &self,
        pipeline_id: PipelineId,
        pull_commit: Commit,
        message: String,
        remote: Remote,
    ) {
        self.send_msg.send(vcs::Message::MergeToStaging(
            pipeline_id, pull_commit, message, remote
        )).unwrap();
    }
    fn move_staging_to_master(
        &self,
        pipeline_id: PipelineId,
        merge_commit: Commit,
    ) {
        self.send_msg.send(vcs::Message::MoveStagingToMaster(
            pipeline_id, merge_commit
        )).unwrap();
    }
}

// TODO: When Rust starts enforcing lifetimes on type aliases,
// use a type alias with something like:
//
//     pub type WorkerPipeline<'cntx> =
//         Pipeline<
//             'cntx,
//             WorkerThread<ci::Event, ci::Message>,
//             WorkerThread<ui::Event, ui::Message>,
//             WorkerThread<vcs::Event, vcs::Message>,
//         >;
//
// That way, we can avoid all these ackward generics in main.
pub struct Pipeline<'cntx, C, U, V>
where C: Ci + 'cntx,
      U: Ui + 'cntx,
      V: Vcs + 'cntx
{
    pub id: PipelineId,
    pub ci: Vec<(CiId, &'cntx C)>,
    pub ui: &'cntx U,
    pub vcs: &'cntx V,
}

#[derive(Clone)]
pub enum Event {
    UiEvent(ui::Event),
    VcsEvent(vcs::Event),
    CiEvent(ci::Event),
}

pub trait GetPipelineId {
    fn pipeline_id<C: PipelinesConfig + ?Sized>(&self, config: &C) -> PipelineId;
}

impl GetPipelineId for Event {
    fn pipeline_id<C: PipelinesConfig + ?Sized>(&self, config: &C) -> PipelineId {
        match *self {
            Event::UiEvent(ref e) => e.pipeline_id(config),
            Event::CiEvent(ref e) => e.pipeline_id(config),
            Event::VcsEvent(ref e) => e.pipeline_id(config),
        }
    }
}

impl<'cntx, C, U, V> Pipeline<'cntx, C, U, V>
where C: Ci + 'cntx,
      U: Ui + 'cntx,
      V: Vcs + 'cntx
{
    pub fn new(
        id: PipelineId,
        ci: Vec<(CiId, &'cntx C)>,
        ui: &'cntx U,
        vcs: &'cntx V,
    ) -> Self {
        Pipeline {
            id: id,
            ci: ci,
            ui: ui,
            vcs: vcs,
        }
    }
    pub fn handle_event<D: Db>(
        &mut self,
        db: &mut D,
        event: Event,
    ) -> Result<(), Box<Error + Send + Sync>> {
        match event {
            Event::UiEvent(ui::Event::Approved(
                pipeline_id,
                pr,
                commit,
                message,
            )) => {
                assert_eq!(&pipeline_id, &self.id);
                let commit = match (
                    commit,
                    try!(db.peek_pending_by_pr(self.id, &pr))
                        .map(|p| p.commit),
                ) {
                    (Some(reviewed_pr), Some(current_pr)) => {
                        if reviewed_pr != current_pr {
                            self.ui.send_result(
                                self.id,
                                pr.clone(),
                                ui::Status::Invalidated,
                            );
                            None
                        } else {
                            Some(reviewed_pr)
                        }
                    }
                    (Some(reviewed_pr), None) => {
                        Some(reviewed_pr)
                    }
                    (None, Some(current_pr)) => {
                        Some(current_pr)
                    }
                    (None, None) => {
                        self.ui.send_result(
                            self.id,
                            pr.clone(),
                            ui::Status::NoCommit,
                        );
                        None
                    }
                };
                if let Some(commit) = commit {
                    self.ui.send_result(
                        self.id,
                        pr.clone(),
                        ui::Status::Approved(commit.clone()),
                    );
                    try!(db.cancel_by_pr(self.id, &pr));
                    try!(db.push_queue(self.id, QueueEntry{
                        commit: commit,
                        pr: pr,
                        message: message,
                    }));
                }
            },
            Event::UiEvent(ui::Event::Opened(
                pipeline_id, pr, commit, title, url
            )) => {
                assert_eq!(&pipeline_id, &self.id);
                try!(db.add_pending(self.id, PendingEntry{
                    commit: commit,
                    pr: pr,
                    title: title,
                    url: url,
                }));
            },
            Event::UiEvent(ui::Event::Changed(
                pipeline_id, pr, commit, title, url
            )) => {
                assert_eq!(&pipeline_id, &self.id);
                if try!(db.cancel_by_pr_different_commit(
                    self.id,
                    &pr,
                    &commit,
                )) {
                    self.ui.send_result(
                        self.id,
                        pr.clone(),
                        ui::Status::Invalidated,
                    );
                }
                try!(db.add_pending(self.id, PendingEntry{
                    commit: commit,
                    pr: pr,
                    title: title,
                    url: url,
                }));
            },
            Event::UiEvent(ui::Event::Closed(pipeline_id, pr)) => {
                assert_eq!(&pipeline_id, &self.id);
                try!(db.take_pending_by_pr(self.id, &pr));
                try!(db.cancel_by_pr(self.id, &pr));
            },
            Event::UiEvent(ui::Event::Canceled(pipeline_id, pr)) => {
                assert_eq!(&pipeline_id, &self.id);
                try!(db.cancel_by_pr(self.id, &pr));
            },
            Event::VcsEvent(vcs::Event::MergedToStaging(
                pipeline_id,
                pull_commit,
                merge_commit
            )) => {
                assert_eq!(&pipeline_id, &self.id);
                if let Some(mut running) = try!(db.take_running(self.id)) {
                    if running.pull_commit != pull_commit {
                        warn!("VCS merged event with wrong commit");
                    } else if running.merge_commit.is_some() {
                        warn!("VCS merged event with running commit");
                    } else if running.canceled {
                        // Drop it on the floor. It's canceled.
                    } else if running.built {
                        warn!("Got merge finished after finished building!");
                    } else {
                        running.merge_commit = Some(merge_commit.clone());
                        for &(ci_id, ci) in &self.ci {
                            try!(db.clear_ci_state(ci_id));
                            ci.start_build(
                                ci_id,
                                merge_commit.clone(),
                            );
                        }
                        self.ui.send_result(
                            self.id,
                            running.pr.clone(),
                            ui::Status::StartingBuild(
                                pull_commit,
                                merge_commit,
                            ),
                        );
                        try!(db.put_running(self.id, running));
                    }
                } else {
                    warn!("VCS merged event with no queued PR");
                }
            },
            Event::VcsEvent(vcs::Event::FailedMergeToStaging(
                pipeline_id,
                pull_commit,
            )) => {
                assert_eq!(&pipeline_id, &self.id);
                if let Some(running) = try!(db.take_running(self.id)) {
                    if running.pull_commit != pull_commit {
                        warn!("VCS merged event with wrong commit");
                    } else if running.merge_commit.is_some() {
                        warn!("VCS merged event with running commit");
                    } else if running.built {
                        warn!("Got merge failed after finished building!");
                    } else if running.canceled {
                        // Drop it on the floor. It's canceled.
                    } else {
                        self.ui.send_result(
                            self.id,
                            running.pr.clone(),
                            ui::Status::Unmergeable(pull_commit),
                        );
                    }
                } else {
                    warn!("VCS merged event with no queued PR");
                }
            },
            Event::CiEvent(ci::Event::BuildStarted(
                _ci_id,
                building_commit,
                url,
            )) => {
                if let Some(running) = try!(db.peek_running(self.id)) {
                    if let Some(merged_commit) = running.merge_commit {
                        if merged_commit != building_commit {
                            warn!("Building a different commit");
                        } else if running.canceled {
                            // Drop it on the floor. It's canceled.
                        } else if running.built {
                            warn!("Got CI build started after done building!");
                        } else {
                            self.ui.send_result(
                                self.id,
                                running.pr.clone(),
                                ui::Status::Testing(
                                    running.pull_commit.clone(),
                                    building_commit,
                                    url,
                                ),
                            );
                        }
                    } else {
                        warn!("Building a commit that never merged");
                    }
                } else {
                    warn!("CI build started event with no queued PR");
                }
            },
            Event::CiEvent(ci::Event::BuildFailed(
                _ci_id,
                built_commit,
                url,
            )) => {
                if let Some(running) = try!(db.take_running(self.id)) {
                    if let Some(ref merged_commit) = running.merge_commit {
                        if merged_commit != &built_commit {
                            warn!("Finished building a different commit");
                            try!(db.put_running(self.id, running.clone()));
                        } else if running.canceled {
                            // Drop it on the floor. It's canceled.
                        } else if running.built {
                            warn!("Got duplicate BuildFailed event");
                            // Put it back
                            try!(db.put_running(self.id, running.clone()));
                        } else {
                            for &(ci_id, _) in &self.ci {
                                if let Some((_state, commit)) = try!(db.get_ci_state(ci_id)) {
                                    assert_eq!(built_commit, commit);
                                    try!(db.clear_ci_state(ci_id));
                                }
                            }
                            self.ui.send_result(
                                self.id,
                                running.pr.clone(),
                                ui::Status::Failure(
                                    running.pull_commit.clone(),
                                    merged_commit.clone(),
                                    url,
                                ),
                            );
                        }
                    } else {
                        warn!("Finished building a commit that never merged");
                    }
                } else {
                    warn!("CI build failed event with no queued PR");
                }
            },
            Event::CiEvent(ci::Event::BuildSucceeded(
                ci_id,
                built_commit,
                url,
            )) => {
                if let Some(mut running) = try!(db.take_running(self.id)) {
                    if let Some(ref merged_commit) = running.merge_commit {
                        if merged_commit != &built_commit {
                            warn!("Finished building a different commit");
                            try!(db.put_running(self.id, running.clone()));
                        } else if running.canceled {
                            // Canceled; drop on the floor.
                        } else if running.built {
                            warn!("Got duplicate BuildSucceeded event");
                            // Put it back.
                            try!(db.put_running(self.id, running.clone()));
                        } else {
                            try!(db.set_ci_state(
                                ci_id,
                                CiState::Succeeded,
                                &built_commit,
                            ));
                            let not_succeeded_count = self.ci.iter()
                                .filter(|&&(ci_id, _)| retry!{{
                                    if let Some((state, commit)) = retry_unwrap!(db.get_ci_state(ci_id)) {
                                        assert_eq!(built_commit, commit);
                                        state != CiState::Succeeded
                                    } else {
                                        true
                                    }
                                }}).count();
                            if not_succeeded_count == 0 {
                                self.vcs.move_staging_to_master(
                                    self.id,
                                    merged_commit.clone(),
                                );
                                self.ui.send_result(
                                    self.id,
                                    running.pr.clone(),
                                    ui::Status::Success(
                                        running.pull_commit.clone(),
                                        merged_commit.clone(),
                                        url,
                                    ),
                                );
                                // Put it back with it marked as built.
                                running.built = true;
                            }
                        }
                    } else {
                        warn!("Finished building a commit that never merged");
                    }
                    try!(db.put_running(self.id, running.clone()));
                } else {
                    warn!("CI build succeeded event with no queued PR");
                }
            },
            Event::VcsEvent(vcs::Event::FailedMoveToMaster(
                pipeline_id,
                merge_commit,
            )) => {
                assert_eq!(&pipeline_id, &self.id);
                if let Some(running) = try!(db.take_running(self.id)) {
                    if let Some(running_merge_commit) = running.merge_commit {
                        if running_merge_commit != merge_commit {
                            warn!("VCS move event with wrong commit");
                        } else if running.canceled {
                            // Drop it on the floor. It's canceled.
                        } else if !running.built {
                            warn!("Failed move to master before built!");
                        } else {
                            self.ui.send_result(
                                self.id,
                                running.pr,
                                ui::Status::Unmoveable(
                                    running.pull_commit,
                                    running_merge_commit,
                                ),
                            );
                        }
                    } else {
                        warn!("VCS move event with commit that never ran");
                    }
                } else {
                    warn!("VCS move event with no queued PR");
                }
            },
            Event::VcsEvent(vcs::Event::MovedToMaster(
                pipeline_id,
                merge_commit,
            )) => {
                assert_eq!(&pipeline_id, &self.id);
                if let Some(running) = try!(db.take_running(self.id)) {
                    if let Some(running_merge_commit) = running.merge_commit {
                        if running_merge_commit != merge_commit {
                            warn!("VCS move event with wrong commit");
                        } else if running.canceled {
                            // Drop it on the floor. It's canceled.
                        } else if !running.built {
                            warn!("Moved to master before done building!");
                        } else {
                            self.ui.send_result(
                                self.id,
                                running.pr,
                                ui::Status::Completed(
                                    running.pull_commit,
                                    running_merge_commit,
                                ),
                            );
                        }
                    } else {
                        warn!("VCS move event with commit that never ran");
                    }
                } else {
                    warn!("VCS move event with no queued PR");
                }
            }
        }
        if try!(db.peek_running(self.id)).is_none() {
            if let Some(next) = try!(db.pop_queue(self.id)) {
                self.vcs.merge_to_staging(
                    self.id,
                    next.commit.clone(),
                    next.message.clone(),
                    next.pr.remote(),
                );
                try!(db.put_running(self.id, RunningEntry{
                    pr: next.pr,
                    message: next.message,
                    pull_commit: next.commit,
                    merge_commit: None,
                    canceled: false,
                    built: false,
                }));
            }
        }
        Ok(())
    }
}

#[cfg(test)] mod test;
