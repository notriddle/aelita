// This file is released under the same terms as Rust itself.

use ci;
use db::{Db, PendingEntry, QueueEntry, RunningEntry};
use std::marker::PhantomData;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;
use ui::{self, Pr};
use vcs::{self, Commit};

pub trait Worker<E: Send + Clone, M: Send + Clone> {
    fn run(&mut self, recv_msg: Receiver<M>, send_event: Sender<E>);
}

pub struct WorkerThread<E: Send + Clone + 'static, M: Send + Clone + 'static> {
    pub recv_event: Receiver<E>,
    pub send_msg: Sender<M>,
}

impl<E: Send + Clone + 'static, M: Send + Clone + 'static> WorkerThread<E, M> {
    pub fn start<T: Worker<E, M> + Send + 'static>(mut worker: T) -> Self {
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

pub trait Ci<C: Commit> {
    fn start_build(&self, pipeline_id: PipelineId, commit: C);
}

impl<C: Commit> Ci<C> for WorkerThread<ci::Event<C>, ci::Message<C>> {
    fn start_build(&self, pipeline_id: PipelineId, commit: C) {
        self.send_msg.send(ci::Message::StartBuild(pipeline_id, commit))
            .unwrap();
    }
}

pub trait Ui<P: Pr> {
    fn send_result(&self, PipelineId, P, ui::Status<P>);
}

impl<P> Ui<P> for WorkerThread<ui::Event<P>, ui::Message<P>>
where P: Pr
{
    fn send_result(
        &self,
        pipeline_id: PipelineId,
        pr: P,
        status: ui::Status<P>,
    ) {
        self.send_msg.send(ui::Message::SendResult(pipeline_id, pr, status))
            .unwrap();
    }
}

pub trait Vcs<C: Commit> {
    fn merge_to_staging(&self, PipelineId, C, String, C::Remote);
    fn move_staging_to_master(&self, PipelineId, C);
}

impl<C: Commit> Vcs<C> for WorkerThread<vcs::Event<C>, vcs::Message<C>> {
    fn merge_to_staging(
        &self,
        pipeline_id: PipelineId,
        pull_commit: C,
        message: String,
        remote: C::Remote
    ) {
        self.send_msg.send(vcs::Message::MergeToStaging(
            pipeline_id, pull_commit, message, remote
        )).unwrap();
    }
    fn move_staging_to_master(
        &self,
        pipeline_id: PipelineId,
        merge_commit: C
    ) {
        self.send_msg.send(vcs::Message::MoveStagingToMaster(
            pipeline_id, merge_commit
        )).unwrap();
    }
}

// TODO: When Rust starts enforcing lifetimes on type aliases,
// use a type alias with something like:
//
//     pub type WorkerPipeline<'cntx, P: Pr + 'static> =
//         Pipeline<
//             'cntx,
//             P,
//             WorkerThread<ci::Event<P::C>, ci::Message<P::C>>,
//             WorkerThread<ui::Event<P::C, P>, ui::Message<P>>,
//             WorkerThread<vcs::Event<P::C>, vcs::Message<P>>,
//         >;
//
// That way, we can avoid all these ackward generics in main.
pub struct Pipeline<'cntx, P, B, U, V>
where P: Pr + 'static,
      B: Ci<P::C> + 'cntx,
      U: Ui<P> + 'cntx,
      V: Vcs<P::C> + 'cntx
{
    pub _pr: PhantomData<P>,
    pub id: PipelineId,
    pub ci: &'cntx B,
    pub ui: &'cntx U,
    pub vcs: &'cntx V,
}

#[derive(Clone)]
pub enum Event<P: Pr + 'static> {
    UiEvent(ui::Event<P>),
    VcsEvent(vcs::Event<P::C>),
    CiEvent(ci::Event<P::C>),
}

pub trait GetPipelineId {
    fn pipeline_id(&self) -> PipelineId;
}

impl<P: Pr + 'static> GetPipelineId for Event<P> {
    fn pipeline_id(&self) -> PipelineId {
        match *self {
            Event::UiEvent(ref e) => e.pipeline_id(),
            Event::CiEvent(ref e) => e.pipeline_id(),
            Event::VcsEvent(ref e) => e.pipeline_id(),
        }
    }
}

impl<'cntx, P, B, U, V> Pipeline<'cntx, P, B, U, V>
where P: Pr + 'static,
      B: Ci<P::C> + 'cntx,
      U: Ui<P> + 'cntx,
      V: Vcs<P::C> + 'cntx
{
    pub fn new(
        id: PipelineId,
        ci: &'cntx B,
        ui: &'cntx U,
        vcs: &'cntx V,
    ) -> Self {
        Pipeline {
            _pr: PhantomData,
            id: id,
            ci: ci,
            ui: ui,
            vcs: vcs,
        }
    }
    pub fn handle_event<D: Db<P>>(
        &mut self,
        db: &mut D,
        event: Event<P>,
    ) {
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
                    db.peek_pending_by_pr(self.id, &pr).map(|p| p.commit),
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
                    db.cancel_by_pr(self.id, &pr);
                    db.push_queue(self.id, QueueEntry{
                        commit: commit,
                        pr: pr,
                        message: message,
                    });
                }
            },
            Event::UiEvent(ui::Event::Opened(
                pipeline_id, pr, commit, title, url
            )) => {
                assert_eq!(&pipeline_id, &self.id);
                db.add_pending(self.id, PendingEntry{
                    commit: commit,
                    pr: pr,
                    title: title,
                    url: url,
                });
            },
            Event::UiEvent(ui::Event::Changed(
                pipeline_id, pr, commit, title, url
            )) => {
                assert_eq!(&pipeline_id, &self.id);
                if db.cancel_by_pr_different_commit(self.id, &pr, &commit) {
                    self.ui.send_result(
                        self.id,
                        pr.clone(),
                        ui::Status::Invalidated,
                    );
                }
                db.add_pending(self.id, PendingEntry{
                    commit: commit,
                    pr: pr,
                    title: title,
                    url: url,
                });
            },
            Event::UiEvent(ui::Event::Closed(pipeline_id, pr)) => {
                assert_eq!(&pipeline_id, &self.id);
                db.take_pending_by_pr(self.id, &pr);
                db.cancel_by_pr(self.id, &pr);
            },
            Event::UiEvent(ui::Event::Canceled(pipeline_id, pr)) => {
                assert_eq!(&pipeline_id, &self.id);
                db.cancel_by_pr(self.id, &pr);
            },
            Event::VcsEvent(vcs::Event::MergedToStaging(
                pipeline_id,
                pull_commit,
                merge_commit
            )) => {
                assert_eq!(&pipeline_id, &self.id);
                if let Some(mut running) = db.take_running(self.id) {
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
                        self.ci.start_build(
                            pipeline_id,
                            merge_commit.clone(),
                        );
                        self.ui.send_result(
                            self.id,
                            running.pr.clone(),
                            ui::Status::StartingBuild(
                                pull_commit,
                                merge_commit,
                            ),
                        );
                        db.put_running(self.id, running);
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
                if let Some(running) = db.take_running(self.id) {
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
                pipeline_id,
                building_commit,
                url,
            )) => {
                assert_eq!(&pipeline_id, &self.id);
                if let Some(running) = db.peek_running(self.id) {
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
                pipeline_id,
                built_commit,
                url,
            )) => {
                assert_eq!(&pipeline_id, &self.id);
                if let Some(running) = db.take_running(self.id) {
                    if let Some(ref merged_commit) = running.merge_commit {
                        if merged_commit != &built_commit {
                            warn!("Finished building a different commit");
                            db.put_running(self.id, running.clone());
                        } else if running.canceled {
                            // Drop it on the floor. It's canceled.
                        } else if running.built {
                            warn!("Got duplicate BuildFailed event");
                            // Put it back
                            db.put_running(self.id, running.clone());
                        } else {
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
                pipeline_id,
                built_commit,
                url,
            )) => {
                assert_eq!(&pipeline_id, &self.id);
                if let Some(mut running) = db.take_running(self.id) {
                    if let Some(ref merged_commit) = running.merge_commit {
                        if merged_commit != &built_commit {
                            warn!("Finished building a different commit");
                            db.put_running(self.id, running.clone());
                        } else if running.canceled {
                            // Canceled; drop on the floor.
                        } else if running.built {
                            warn!("Got duplicate BuildSucceeded event");
                            // Put it back.
                            db.put_running(self.id, running.clone());
                        } else {
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
                            db.put_running(self.id, running.clone());
                        }
                    } else {
                        warn!("Finished building a commit that never merged");
                    }
                } else {
                    warn!("CI build succeeded event with no queued PR");
                }
            },
            Event::VcsEvent(vcs::Event::FailedMoveToMaster(
                pipeline_id,
                merge_commit,
            )) => {
                assert_eq!(&pipeline_id, &self.id);
                if let Some(running) = db.take_running(self.id) {
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
                if let Some(running) = db.take_running(self.id) {
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
        if db.peek_running(self.id).is_none() {
            if let Some(next) = db.pop_queue(self.id) {
                self.vcs.merge_to_staging(
                    self.id,
                    next.commit.clone(),
                    next.message.clone(),
                    next.pr.remote(),
                );
                db.put_running(self.id, RunningEntry{
                    pr: next.pr,
                    message: next.message,
                    pull_commit: next.commit,
                    merge_commit: None,
                    canceled: false,
                    built: false,
                });
            }
        }
    }
}

#[cfg(test)] mod test;
