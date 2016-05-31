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

pub trait Ui<C: Commit, P: Pr> {
    fn send_result(&self, pipeline_id: PipelineId, pr: P, status: ui::Status);
}

impl<C, P> Ui<C, P> for WorkerThread<ui::Event<C, P>, ui::Message<P>>
where C: Commit,
      P: Pr
{
    fn send_result(&self, pipeline_id: PipelineId, pr: P, status: ui::Status) {
        self.send_msg.send(ui::Message::SendResult(pipeline_id, pr, status))
            .unwrap();
    }
}

pub trait Vcs<C: Commit> {
    fn merge_to_staging(&self, PipelineId, C, String, String);
    fn move_staging_to_master(&self, PipelineId, C);
}

impl<C: Commit> Vcs<C> for WorkerThread<vcs::Event<C>, vcs::Message<C>> {
    fn merge_to_staging(
        &self,
        pipeline_id: PipelineId,
        pull_commit: C,
        message: String,
        remote: String
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
//     pub type WorkerPipeline<'cntx, C: Commit + 'static, P: Pr + 'static> =
//         Pipeline<
//             'cntx,
//             C,
//             P,
//             WorkerThread<ci::Event<C>, ci::Message<C>>,
//             WorkerThread<ui::Event<C, P>, ui::Message<P>>,
//             WorkerThread<vcs::Event<C>, vcs::Message<P>>,
//         >;
//
// That way, we can avoid all these ackward generics in main.
pub struct Pipeline<'cntx, C, P, B, U, V>
where C: Commit + 'static,
      P: Pr + 'static,
      B: Ci<C> + 'cntx,
      U: Ui<C, P> + 'cntx,
      V: Vcs<C> + 'cntx
{
    pub _commit: PhantomData<C>,
    pub _pr: PhantomData<P>,
    pub id: PipelineId,
    pub ci: &'cntx B,
    pub ui: &'cntx U,
    pub vcs: &'cntx V,
}

#[derive(Clone)]
pub enum Event<C: Commit + 'static, P: Pr + 'static> {
    UiEvent(ui::Event<C, P>),
    VcsEvent(vcs::Event<C>),
    CiEvent(ci::Event<C>),
}

pub trait GetPipelineId {
    fn pipeline_id(&self) -> PipelineId;
}

impl<C: Commit + 'static, P: Pr + 'static> GetPipelineId for Event<C, P> {
    fn pipeline_id(&self) -> PipelineId {
        match *self {
            Event::UiEvent(ref e) => e.pipeline_id(),
            Event::CiEvent(ref e) => e.pipeline_id(),
            Event::VcsEvent(ref e) => e.pipeline_id(),
        }
    }
}

impl<'cntx, C, P, B, U, V> Pipeline<'cntx, C, P, B, U, V>
where C: Commit + 'static,
      P: Pr + 'static,
      B: Ci<C> + 'cntx,
      U: Ui<C, P> + 'cntx,
      V: Vcs<C> + 'cntx
{
    pub fn new(
        id: PipelineId,
        ci: &'cntx B,
        ui: &'cntx U,
        vcs: &'cntx V,
    ) -> Self {
        Pipeline {
            _commit: PhantomData,
            _pr: PhantomData,
            id: id,
            ci: ci,
            ui: ui,
            vcs: vcs,
        }
    }
    pub fn handle_event<D: Db<C, P>>(
        &mut self,
        db: &mut D,
        event: Event<C, P>,
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
            Event::UiEvent(ui::Event::Opened(pipeline_id, pr, commit)) => {
                assert_eq!(&pipeline_id, &self.id);
                db.add_pending(self.id, PendingEntry{
                    commit: commit,
                    pr: pr,
                });
            },
            Event::UiEvent(ui::Event::Changed(pipeline_id, pr, commit)) => {
                assert_eq!(&pipeline_id, &self.id);
                if db.cancel_by_pr_different_commit(self.id, &pr, &commit) {
                    self.ui.send_result(
                        self.id,
                        pr.clone(),
                        ui::Status::Invalidated,
                    );
                }
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
                        panic!("VCS merged event with wrong commit");
                    } else if running.merge_commit.is_some() {
                        panic!("VCS merged event with running commit");
                    } else if running.canceled {
                        // Drop it on the floor. It's canceled.
                    } else {
                        running.merge_commit = Some(merge_commit.clone());
                        self.ci.start_build(
                            pipeline_id,
                            merge_commit,
                        );
                        self.ui.send_result(
                            self.id,
                            running.pr.clone(),
                            ui::Status::InProgress,
                        );
                        db.put_running(self.id, running);
                    }
                } else {
                    panic!("VCS merged event with no queued PR");
                }
            },
            Event::VcsEvent(vcs::Event::FailedMergeToStaging(
                pipeline_id,
                pull_commit,
            )) => {
                assert_eq!(&pipeline_id, &self.id);
                if let Some(running) = db.take_running(self.id) {
                    if running.pull_commit != pull_commit {
                        panic!("VCS merged event with wrong commit");
                    } else if running.merge_commit.is_some() {
                        panic!("VCS merged event with running commit");
                    } else if running.canceled {
                        // Drop it on the floor. It's canceled.
                    } else {
                        self.ui.send_result(
                            self.id,
                            running.pr.clone(),
                            ui::Status::Unmergeable,
                        );
                    }
                } else {
                    panic!("VCS merged event with no queued PR");
                }
            },
            Event::CiEvent(ci::Event::BuildFailed(
                pipeline_id,
                built_commit,
            )) => {
                assert_eq!(&pipeline_id, &self.id);
                if let Some(running) = db.take_running(self.id) {
                    if let Some(merged_commit) = running.merge_commit {
                        if merged_commit != built_commit {
                            panic!("Finished building a different commit");
                        } else if running.canceled {
                            // Drop it on the floor. It's canceled.
                        } else {
                            self.ui.send_result(
                                self.id,
                                running.pr.clone(),
                                ui::Status::Failure,
                            );
                        }
                    } else {
                        panic!("Finished building a commit that never merged");
                    }
                } else {
                    panic!("VCS merged event with no queued PR");
                }
            },
            Event::CiEvent(ci::Event::BuildSucceeded(
                pipeline_id,
                built_commit,
            )) => {
                assert_eq!(&pipeline_id, &self.id);
                if let Some(running) = db.peek_running(self.id) {
                    if let Some(merged_commit) = running.merge_commit {
                        if &merged_commit != &built_commit {
                            panic!("Finished building a different commit")
                        } else if running.canceled {
                            // Canceled; drop on the floor.
                            let running2 = db.take_running(self.id)
                                .expect("Just peeked");
                            assert_eq!(
                                running2.merge_commit,
                                Some(merged_commit)
                            );
                        } else {
                            self.vcs.move_staging_to_master(
                                self.id,
                                merged_commit,
                            );
                        }
                    } else {
                        panic!("Finished building a commit that never merged");
                    }
                } else {
                    panic!("VCS merged event with no queued PR");
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
                            panic!("VCS move event with wrong commit");
                        } else if running.canceled {
                            // Drop it on the floor. It's canceled.
                        } else {
                            self.ui.send_result(
                                self.id,
                                running.pr,
                                ui::Status::Unmoveable,
                            );
                        }
                    } else {
                        panic!("VCS move event with commit that never ran");
                    }
                } else {
                    panic!("VCS move event with no queued PR");
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
                            panic!("VCS move event with wrong commit");
                        } else if running.canceled {
                            // Drop it on the floor. It's canceled.
                        } else {
                            self.ui.send_result(
                                self.id,
                                running.pr,
                                ui::Status::Success,
                            );
                        }
                    } else {
                        panic!("VCS move event with commit that never ran");
                    }
                } else {
                    panic!("VCS move event with no queued PR");
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
                });
            }
        }
    }
}

#[cfg(test)] mod test {

use super::{Ci, Vcs, Ui};
use ci;
use db::{Db, PendingEntry, QueueEntry, RunningEntry};
use pipeline::{Event, Pipeline, PipelineId};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::fmt::{self, Debug, Display};
use std::marker::PhantomData;
use std::mem;
use std::str::FromStr;
use ui::{self, Pr};
use vcs::{self, Commit};
use void::Void;

struct MemoryDb<C: Commit, P: Pr> {
    queue: VecDeque<QueueEntry<C, P>>,
    running: Option<RunningEntry<C, P>>,
    pending: Vec<PendingEntry<C, P>>,
}

impl<C: Commit, P: Pr> MemoryDb<C, P> {
    fn new() -> Self {
        MemoryDb{
            queue: VecDeque::new(),
            running: None,
            pending: Vec::new(),
        }
    }
}

impl<C: Commit, P: Pr> Db<C, P> for MemoryDb<C, P> {
    fn push_queue(&mut self, _: PipelineId, entry: QueueEntry<C, P>) {
        self.queue.push_back(entry);
    }
    fn pop_queue(&mut self, _: PipelineId) -> Option<QueueEntry<C, P>> {
        self.queue.pop_front()
    }
    fn put_running(&mut self, _: PipelineId, entry: RunningEntry<C, P>) {
        self.running = Some(entry);
    }
    fn take_running(&mut self, _: PipelineId) -> Option<RunningEntry<C, P>> {
        mem::replace(&mut self.running, None)
    }
    fn peek_running(&mut self, _: PipelineId) -> Option<RunningEntry<C, P>> {
        self.running.clone()
    }
    fn add_pending(&mut self, _: PipelineId, mut entry: PendingEntry<C, P>) {
        let mut replaced = false;
        for ref mut entry2 in self.pending.iter_mut() {
            if entry2.pr == entry.pr {
                mem::replace(entry2, &mut entry);
                replaced = true;
                break;
            }
        }
        if !replaced {
            self.pending.push(entry);
        }
    }
    fn peek_pending_by_pr(
        &mut self,
        _: PipelineId,
        pr: &P,
    ) -> Option<PendingEntry<C, P>> {
        for entry in &self.pending {
            if entry.pr == *pr {
                return Some(entry.clone());
            }
        }
        None
    }
    fn take_pending_by_pr(
        &mut self,
        _: PipelineId,
        pr: &P,
    ) -> Option<PendingEntry<C, P>> {
        let mut entry_i = None;
        for (i, entry) in self.pending.iter().enumerate() {
            if entry.pr == *pr {
                entry_i = Some(i);
                break;
            }
        }
        entry_i.map(|entry_i| self.pending.remove(entry_i))
    }
    fn cancel_by_pr(&mut self, _: PipelineId, pr: &P) {
        let queue = mem::replace(&mut self.queue, VecDeque::new());
        let filtered = queue.into_iter().filter(|entry| entry.pr != *pr);
        self.queue.extend(filtered);
        if let Some(ref mut running) = self.running {
            if running.pr == *pr {
                running.canceled = true;
            }
        }
    }
    fn cancel_by_pr_different_commit(
        &mut self,
        _: PipelineId,
        pr: &P,
        commit: &C
    ) -> bool {
        let len_orig = self.queue.len();
        let queue = mem::replace(&mut self.queue, VecDeque::new());
        let filtered = queue.into_iter().filter(|entry|
            entry.pr != *pr || entry.commit == *commit
        );
        self.queue.extend(filtered);
        let mut canceled = len_orig != self.queue.len();
        if let Some(ref mut running) = self.running {
            if running.pr == *pr && running.pull_commit != *commit {
                running.canceled = true;
                canceled = true;
            }
        }
        canceled
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(unused)]
enum MemoryCommit {
    A, B, C, D, E, F, G, H, I, J, K, L, M,
    N, O, P, Q, R, S, T, U, V, W, X, Y, Z
}
impl Commit for MemoryCommit {}
impl FromStr for MemoryCommit {
    type Err = Void;
    fn from_str(_: &str) -> Result<MemoryCommit, Void> {
        Ok(MemoryCommit::A)
    }
}
impl Display for MemoryCommit {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        <Self as Debug>::fmt(self, f)
    }
}
impl Into<String> for MemoryCommit {
    fn into(self) -> String {
        self.to_string()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(unused)]
enum MemoryPr {
    A, B, C, D, E, F, G, H, I, J, K, L, M,
    N, O, P, Q, R, S, T, U, V, W, X, Y, Z
}
impl Pr for MemoryPr {
    fn remote(&self) -> String {
        "".to_owned()
    }
}
impl FromStr for MemoryPr {
    type Err = Void;
    fn from_str(_: &str) -> Result<MemoryPr, Void> {
        Ok(MemoryPr::A)
    }
}
impl Display for MemoryPr {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        <Self as Debug>::fmt(self, f)
    }
}
impl Into<String> for MemoryPr {
    fn into(self) -> String {
        self.to_string()
    }
}

struct MemoryUi {
    results: Vec<(MemoryPr, ui::Status)>,
}
impl MemoryUi {
    fn new() -> RefCell<MemoryUi> {
        RefCell::new(MemoryUi{
            results: Vec::new(),
        })
    }
}
impl Ui<MemoryCommit, MemoryPr> for RefCell<MemoryUi> {
    fn send_result(&self, _: PipelineId, pr: MemoryPr, status: ui::Status) {
        self.borrow_mut().results.push((pr, status));
    }
}

struct MemoryVcs {
    staging: Option<MemoryCommit>,
    master: Option<MemoryCommit>,
}
impl MemoryVcs {
    fn new() -> RefCell<MemoryVcs> {
        RefCell::new(MemoryVcs{
            staging: None,
            master: None,
        })
    }
}
impl Vcs<MemoryCommit> for RefCell<MemoryVcs> {
    fn merge_to_staging(
        &self,
        _: PipelineId,
        pull_commit: MemoryCommit,
        _message: String,
        _remote: String,
    ) {
        self.borrow_mut().staging = Some(pull_commit)
    }
    fn move_staging_to_master(&self, _: PipelineId, commit: MemoryCommit) {
        self.borrow_mut().master = Some(commit)
    }
}

struct MemoryCi {
    build: Option<MemoryCommit>,
}
impl MemoryCi {
    fn new() -> RefCell<MemoryCi> {
        RefCell::new(MemoryCi{
            build: None,
        })
    }
}
impl Ci<MemoryCommit> for RefCell<MemoryCi> {
    fn start_build(&self, _: PipelineId, commit: MemoryCommit) {
        self.borrow_mut().build = Some(commit);
    }
}

fn handle_event(
    ui: &mut RefCell<MemoryUi>,
    vcs: &mut RefCell<MemoryVcs>,
    ci: &mut RefCell<MemoryCi>,
    db: &mut MemoryDb<MemoryCommit, MemoryPr>,
    event: Event<MemoryCommit, MemoryPr>,
) {
    Pipeline{
        _pr: PhantomData,
        _commit: PhantomData,
        ui: ui,
        vcs: vcs,
        ci: ci,
        id: PipelineId(0),
    }.handle_event(db, event);
}


#[test]
fn handle_add_to_queue() {
    let mut ui = MemoryUi::new();
    let mut vcs = MemoryVcs::new();
    let mut ci = MemoryCi::new();
    let mut db = MemoryDb::new();
    handle_event(
        &mut ui,
        &mut vcs,
        &mut ci,
        &mut db,
        Event::UiEvent(ui::Event::Approved(
            PipelineId(0),
            MemoryPr::A,
            Some(MemoryCommit::A),
            "Message!".to_owned(),
        )),
    );
    assert_eq!(db.running.unwrap().pull_commit, MemoryCommit::A);
    assert!(db.queue.is_empty());
    assert_eq!(vcs.borrow().staging.unwrap(), MemoryCommit::A);
}

#[test]
fn handle_add_to_queue_by_pending_none() {
    let mut ui = MemoryUi::new();
    let mut vcs = MemoryVcs::new();
    let mut ci = MemoryCi::new();
    let mut db = MemoryDb::new();
    handle_event(
        &mut ui,
        &mut vcs,
        &mut ci,
        &mut db,
        Event::UiEvent(ui::Event::Approved(
            PipelineId(0),
            MemoryPr::A,
            None,
            "Message!".to_owned(),
        )),
    );
    assert!(db.running.is_none());
    assert!(db.queue.is_empty());
    assert!(vcs.borrow().staging.is_none());
    assert_eq!(ui.borrow().results[0].1, ui::Status::NoCommit);
}

#[test]
fn handle_add_to_queue_by_pending_some() {
    let mut ui = MemoryUi::new();
    let mut vcs = MemoryVcs::new();
    let mut ci = MemoryCi::new();
    let mut db = MemoryDb::new();
    handle_event(
        &mut ui,
        &mut vcs,
        &mut ci,
        &mut db,
        Event::UiEvent(ui::Event::Opened(
            PipelineId(0),
            MemoryPr::A,
            MemoryCommit::A,
        )),
    );
    handle_event(
        &mut ui,
        &mut vcs,
        &mut ci,
        &mut db,
        Event::UiEvent(ui::Event::Approved(
            PipelineId(0),
            MemoryPr::A,
            None,
            "Message!".to_owned(),
        )),
    );
    assert_eq!(db.running.unwrap().pull_commit, MemoryCommit::A);
    assert!(db.queue.is_empty());
    assert_eq!(vcs.borrow().staging.unwrap(), MemoryCommit::A);
}

#[test]
fn handle_add_two_to_queue() {
    let mut ui = MemoryUi::new();
    let mut vcs = MemoryVcs::new();
    let mut ci = MemoryCi::new();
    let mut db = MemoryDb::new();
    handle_event(
        &mut ui,
        &mut vcs,
        &mut ci,
        &mut db,
        Event::UiEvent(ui::Event::Approved(
            PipelineId(0),
            MemoryPr::A,
            Some(MemoryCommit::A),
            "Message!".to_owned(),
        ))
    );
    handle_event(
        &mut ui,
        &mut vcs,
        &mut ci,
        &mut db,
        Event::UiEvent(ui::Event::Approved(
            PipelineId(0),
            MemoryPr::B,
            Some(MemoryCommit::B),
            "Message!".to_owned(),
        ))
    );
    assert!(!db.running.clone().unwrap().canceled);
    assert_eq!(db.running.unwrap().pull_commit, MemoryCommit::A);
    assert_eq!(db.queue.front().unwrap().commit, MemoryCommit::B);
    assert_eq!(vcs.borrow().staging.unwrap(), MemoryCommit::A);
}

#[test]
fn handle_add_two_same_pr_to_queue() {
    let mut ui = MemoryUi::new();
    let mut vcs = MemoryVcs::new();
    let mut ci = MemoryCi::new();
    let mut db = MemoryDb::new();
    handle_event(
        &mut ui,
        &mut vcs,
        &mut ci,
        &mut db,
        Event::UiEvent(ui::Event::Approved(
            PipelineId(0),
            MemoryPr::A,
            Some(MemoryCommit::A),
            "Message!".to_owned(),
        ))
    );
    handle_event(
        &mut ui,
        &mut vcs,
        &mut ci,
        &mut db,
        Event::UiEvent(ui::Event::Approved(
            PipelineId(0),
            MemoryPr::A,
            Some(MemoryCommit::B),
            "Message!".to_owned(),
        ))
    );
    assert!(db.running.clone().unwrap().canceled);
    assert_eq!(db.running.unwrap().pull_commit, MemoryCommit::A);
    assert_eq!(db.queue.front().unwrap().commit, MemoryCommit::B);
    assert_eq!(vcs.borrow().staging.unwrap(), MemoryCommit::A);
}

#[test]
fn handle_add_three_same_pr_to_queue() {
    let mut ui = MemoryUi::new();
    let mut vcs = MemoryVcs::new();
    let mut ci = MemoryCi::new();
    let mut db = MemoryDb::new();
    handle_event(
        &mut ui,
        &mut vcs,
        &mut ci,
        &mut db,
        Event::UiEvent(ui::Event::Approved(
            PipelineId(0),
            MemoryPr::A,
            Some(MemoryCommit::A),
            "Message!".to_owned(),
        ))
    );
    handle_event(
        &mut ui,
        &mut vcs,
        &mut ci,
        &mut db,
        Event::UiEvent(ui::Event::Approved(
            PipelineId(0),
            MemoryPr::A,
            Some(MemoryCommit::B),
            "Message!".to_owned(),
        ))
    );
    handle_event(
        &mut ui,
        &mut vcs,
        &mut ci,
        &mut db,
        Event::UiEvent(ui::Event::Approved(
            PipelineId(0),
            MemoryPr::A,
            Some(MemoryCommit::C),
            "Message!".to_owned(),
        ))
    );
    assert!(db.running.clone().unwrap().canceled);
    assert_eq!(db.running.unwrap().pull_commit, MemoryCommit::A);
    assert_eq!(db.queue.front().unwrap().commit, MemoryCommit::C);
    assert_eq!(vcs.borrow().staging.unwrap(), MemoryCommit::A);
}

#[test]
fn handle_merge_failed_notify_user() {
    let mut ui = MemoryUi::new();
    let mut vcs = MemoryVcs::new();
    let mut ci = MemoryCi::new();
    let mut db = MemoryDb::new();
    db.put_running(PipelineId(0), RunningEntry{
        pull_commit: MemoryCommit::A,
        merge_commit: None,
        pr: MemoryPr::A,
        message: "MSG!".to_owned(),
        canceled: false,
    });
    handle_event(
        &mut ui,
        &mut vcs,
        &mut ci,
        &mut db,
        Event::VcsEvent(vcs::Event::FailedMergeToStaging(
            PipelineId(0),
            MemoryCommit::A
        ))
    );
    assert!(db.running.is_none());
    assert!(db.queue.is_empty());
    assert!(ci.borrow().build.is_none());
    assert!(vcs.borrow().master.is_none());
    assert_eq!(
        ui.borrow().results,
        vec![(MemoryPr::A, ui::Status::Unmergeable)]
    );
}

#[test]
fn handle_merge_failed_notify_user_merge_next_commit() {
    let mut ui = MemoryUi::new();
    let mut vcs = MemoryVcs::new();
    let mut ci = MemoryCi::new();
    let mut db = MemoryDb::new();
    db.put_running(PipelineId(0), RunningEntry{
        pull_commit: MemoryCommit::A,
        merge_commit: None,
        pr: MemoryPr::A,
        message: "MSG!".to_owned(),
        canceled: false,
    });
    db.push_queue(PipelineId(0), QueueEntry{
        commit: MemoryCommit::C,
        pr: MemoryPr::B,
        message: "M!".to_owned(),
    });
    handle_event(
        &mut ui,
        &mut vcs,
        &mut ci,
        &mut db,
        Event::VcsEvent(vcs::Event::FailedMergeToStaging(
            PipelineId(0),
            MemoryCommit::A
        ))
    );
    assert_eq!(db.running.unwrap(), RunningEntry{
        pull_commit: MemoryCommit::C,
        merge_commit: None,
        pr: MemoryPr::B,
        message: "M!".to_owned(),
        canceled: false,
    });
    assert!(db.queue.is_empty());
    assert!(ci.borrow().build.is_none());
    assert!(vcs.borrow().master.is_none());
    assert_eq!(vcs.borrow().staging.unwrap(), MemoryCommit::C);
    assert_eq!(
        ui.borrow().results,
        vec![(MemoryPr::A, ui::Status::Unmergeable)]
    );
}

#[test]
fn handle_merge_succeeded_notify_user_start_ci() {
    let mut ui = MemoryUi::new();
    let mut vcs = MemoryVcs::new();
    let mut ci = MemoryCi::new();
    let mut db = MemoryDb::new();
    db.put_running(PipelineId(0), RunningEntry{
        pull_commit: MemoryCommit::A,
        merge_commit: None,
        pr: MemoryPr::A,
        message: "MSG!".to_owned(),
        canceled: false,
    });
    vcs.borrow_mut().staging = Some(MemoryCommit::B);
    handle_event(
        &mut ui,
        &mut vcs,
        &mut ci,
        &mut db,
        Event::VcsEvent(vcs::Event::MergedToStaging(
            PipelineId(0),
            MemoryCommit::A,
            MemoryCommit::B
        )),
    );
    assert_eq!(db.running.unwrap(), RunningEntry{
        pull_commit: MemoryCommit::A,
        merge_commit: Some(MemoryCommit::B),
        pr: MemoryPr::A,
        message: "MSG!".to_owned(),
        canceled: false,
    });
    assert!(db.queue.is_empty());
    assert_eq!(ci.borrow().build.unwrap(), MemoryCommit::B);
    assert!(vcs.borrow().master.is_none());
    assert_eq!(vcs.borrow().staging.unwrap(), MemoryCommit::B);
    assert_eq!(
        ui.borrow().results,
        vec![(MemoryPr::A, ui::Status::InProgress)]
    );
}

#[test]
fn handle_ci_failed_notify_user() {
    let mut ui = MemoryUi::new();
    let mut vcs = MemoryVcs::new();
    let mut ci = MemoryCi::new();
    let mut db = MemoryDb::new();
    db.put_running(PipelineId(0), RunningEntry{
        pull_commit: MemoryCommit::A,
        merge_commit: Some(MemoryCommit::B),
        pr: MemoryPr::A,
        message: "MSG!".to_owned(),
        canceled: false,
    });
    vcs.borrow_mut().staging = Some(MemoryCommit::B);
    handle_event(
        &mut ui,
        &mut vcs,
        &mut ci,
        &mut db,
        Event::CiEvent(ci::Event::BuildFailed(
            PipelineId(0),
            MemoryCommit::B,
        ))
    );
    assert!(db.running.is_none());
    assert!(db.queue.is_empty());
    assert!(vcs.borrow().master.is_none());
    assert_eq!(vcs.borrow().staging.unwrap(), MemoryCommit::B);
    assert_eq!(
        ui.borrow().results,
        vec![(MemoryPr::A, ui::Status::Failure)]
    );
}

#[test]
fn handle_ci_failed_notify_user_next_commit() {
    let mut ui = MemoryUi::new();
    let mut vcs = MemoryVcs::new();
    let mut ci = MemoryCi::new();
    let mut db = MemoryDb::new();
    db.put_running(PipelineId(0), RunningEntry{
        pull_commit: MemoryCommit::A,
        merge_commit: Some(MemoryCommit::B),
        pr: MemoryPr::A,
        message: "MSG!".to_owned(),
        canceled: false,
    });
    db.push_queue(PipelineId(0), QueueEntry{
        commit: MemoryCommit::C,
        pr: MemoryPr::B,
        message: "M!".to_owned(),
    });
    vcs.borrow_mut().staging = Some(MemoryCommit::B);
    handle_event(
        &mut ui,
        &mut vcs,
        &mut ci,
        &mut db,
        Event::CiEvent(ci::Event::BuildFailed(
            PipelineId(0),
            MemoryCommit::B,
        ))
    );
    assert_eq!(db.running.unwrap(), RunningEntry{
        pull_commit: MemoryCommit::C,
        merge_commit: None,
        pr: MemoryPr::B,
        message: "M!".to_owned(),
        canceled: false,
    });
    assert!(db.queue.is_empty());
    assert!(vcs.borrow().master.is_none());
    assert_eq!(vcs.borrow().staging.unwrap(), MemoryCommit::C);
    assert_eq!(
        ui.borrow().results,
        vec![(MemoryPr::A, ui::Status::Failure)]
    );
}

#[test]
fn handle_ci_succeeded_move_to_master() {
    let mut ui = MemoryUi::new();
    let mut vcs = MemoryVcs::new();
    let mut ci = MemoryCi::new();
    let mut db = MemoryDb::new();
    db.put_running(PipelineId(0), RunningEntry{
        pull_commit: MemoryCommit::A,
        merge_commit: Some(MemoryCommit::B),
        pr: MemoryPr::A,
        canceled: false,
        message: "MSG!".to_owned(),
    });
    vcs.borrow_mut().staging = Some(MemoryCommit::B);
    handle_event(
        &mut ui,
        &mut vcs,
        &mut ci,
        &mut db,
        Event::CiEvent(ci::Event::BuildSucceeded(
            PipelineId(0),
            MemoryCommit::B,
        ))
    );
    assert!(db.running.is_some());
    assert!(db.queue.is_empty());
    assert_eq!(vcs.borrow().master.unwrap(), MemoryCommit::B);
    assert_eq!(
        ui.borrow().results,
        vec![]
    );
}

#[test]
fn handle_move_failed_notify_user() {
    let mut ui = MemoryUi::new();
    let mut vcs = MemoryVcs::new();
    let mut ci = MemoryCi::new();
    let mut db = MemoryDb::new();
    db.put_running(PipelineId(0), RunningEntry{
        pull_commit: MemoryCommit::A,
        merge_commit: Some(MemoryCommit::B),
        pr: MemoryPr::A,
        canceled: false,
        message: "MSG!".to_owned(),
    });
    vcs.borrow_mut().staging = Some(MemoryCommit::B);
    handle_event(
        &mut ui,
        &mut vcs,
        &mut ci,
        &mut db,
        Event::VcsEvent(vcs::Event::FailedMoveToMaster(
            PipelineId(0),
            MemoryCommit::B,
        ))
    );
    assert!(db.running.is_none());
    assert!(db.queue.is_empty());
    assert!(vcs.borrow().master.is_none());
    assert_eq!(vcs.borrow().staging.unwrap(), MemoryCommit::B);
    assert_eq!(
        ui.borrow().results,
        vec![(MemoryPr::A, ui::Status::Unmoveable)]
    );
}

#[test]
fn handle_move_failed_notify_user_next_commit() {
    let mut ui = MemoryUi::new();
    let mut vcs = MemoryVcs::new();
    let mut ci = MemoryCi::new();
    let mut db = MemoryDb::new();
    db.put_running(PipelineId(0), RunningEntry{
        pull_commit: MemoryCommit::A,
        merge_commit: Some(MemoryCommit::B),
        pr: MemoryPr::A,
        canceled: false,
        message: "MSG!".to_owned(),
    });
    db.push_queue(PipelineId(0), QueueEntry{
        commit: MemoryCommit::C,
        pr: MemoryPr::B,
        message: "M!".to_owned(),
    });
    vcs.borrow_mut().staging = Some(MemoryCommit::B);
    handle_event(
        &mut ui,
        &mut vcs,
        &mut ci,
        &mut db,
        Event::VcsEvent(vcs::Event::FailedMoveToMaster(
            PipelineId(0),
            MemoryCommit::B,
        ))
    );
    assert_eq!(db.running.unwrap(), RunningEntry{
        pull_commit: MemoryCommit::C,
        merge_commit: None,
        pr: MemoryPr::B,
        message: "M!".to_owned(),
        canceled: false,
    });
    assert!(db.queue.is_empty());
    assert!(vcs.borrow().master.is_none());
    assert_eq!(vcs.borrow().staging.unwrap(), MemoryCommit::C);
    assert_eq!(
        ui.borrow().results,
        vec![(MemoryPr::A, ui::Status::Unmoveable)]
    );
}

#[test]
fn handle_move_succeeded_notify_user() {
    let mut ui = MemoryUi::new();
    let mut vcs = MemoryVcs::new();
    let mut ci = MemoryCi::new();
    let mut db = MemoryDb::new();
    db.put_running(PipelineId(0), RunningEntry{
        pull_commit: MemoryCommit::A,
        merge_commit: Some(MemoryCommit::B),
        pr: MemoryPr::A,
        message: "MSG!".to_owned(),
        canceled: false,
    });
    vcs.borrow_mut().staging = Some(MemoryCommit::B);
    handle_event(
        &mut ui,
        &mut vcs,
        &mut ci,
        &mut db,
        Event::VcsEvent(vcs::Event::MovedToMaster(
            PipelineId(0),
            MemoryCommit::B,
        ))
    );
    assert!(db.running.is_none());
    assert!(db.queue.is_empty());
    assert!(vcs.borrow().master.is_none());
    assert_eq!(
        ui.borrow().results,
        vec![(MemoryPr::A, ui::Status::Success)]
    );
}

#[test]
fn handle_move_succeeded_notify_user_next_commit() {
    let mut ui = MemoryUi::new();
    let mut vcs = MemoryVcs::new();
    let mut ci = MemoryCi::new();
    let mut db = MemoryDb::new();
    db.put_running(PipelineId(0), RunningEntry{
        pull_commit: MemoryCommit::A,
        merge_commit: Some(MemoryCommit::B),
        pr: MemoryPr::A,
        message: "MSG!".to_owned(),
        canceled: false,
    });
    db.push_queue(PipelineId(0), QueueEntry{
        commit: MemoryCommit::C,
        pr: MemoryPr::B,
        message: "M!".to_owned(),
    });
    vcs.borrow_mut().staging = Some(MemoryCommit::B);
    handle_event(
        &mut ui,
        &mut vcs,
        &mut ci,
        &mut db,
        Event::VcsEvent(vcs::Event::MovedToMaster(
            PipelineId(0),
            MemoryCommit::B,
        ))
    );
    assert_eq!(db.running.unwrap(), RunningEntry{
        pull_commit: MemoryCommit::C,
        merge_commit: None,
        pr: MemoryPr::B,
        message: "M!".to_owned(),
        canceled: false,
    });
    assert!(db.queue.is_empty());
    assert!(vcs.borrow().master.is_none());
    assert_eq!(vcs.borrow().staging.unwrap(), MemoryCommit::C);
    assert_eq!(
        ui.borrow().results,
        vec![(MemoryPr::A, ui::Status::Success)]
    );
}

#[test]
fn handle_ui_cancel() {
    let mut ui = MemoryUi::new();
    let mut vcs = MemoryVcs::new();
    let mut ci = MemoryCi::new();
    let mut db = MemoryDb::new();
    db.put_running(PipelineId(0), RunningEntry{
        pull_commit: MemoryCommit::A,
        merge_commit: Some(MemoryCommit::B),
        pr: MemoryPr::A,
        message: "MSG!".to_owned(),
        canceled: false,
    });
    handle_event(
        &mut ui,
        &mut vcs,
        &mut ci,
        &mut db,
        Event::UiEvent(ui::Event::Canceled(
            PipelineId(0),
            MemoryPr::A
        ))
    );
    assert_eq!(db.running.unwrap(), RunningEntry{
        pull_commit: MemoryCommit::A,
        merge_commit: Some(MemoryCommit::B),
        pr: MemoryPr::A,
        canceled: true,
        message: "MSG!".to_owned(),
    });
}

#[test]
fn handle_ui_changed_cancel() {
    let mut ui = MemoryUi::new();
    let mut vcs = MemoryVcs::new();
    let mut ci = MemoryCi::new();
    let mut db = MemoryDb::new();
    db.put_running(PipelineId(0), RunningEntry{
        pull_commit: MemoryCommit::A,
        merge_commit: Some(MemoryCommit::B),
        pr: MemoryPr::A,
        message: "MSG!".to_owned(),
        canceled: false,
    });
    handle_event(
        &mut ui,
        &mut vcs,
        &mut ci,
        &mut db,
        Event::UiEvent(ui::Event::Changed(
            PipelineId(0),
            MemoryPr::A,
            MemoryCommit::C,
        ))
    );
    assert_eq!(db.running.unwrap(), RunningEntry{
        pull_commit: MemoryCommit::A,
        merge_commit: Some(MemoryCommit::B),
        pr: MemoryPr::A,
        canceled: true,
        message: "MSG!".to_owned(),
    });
}

#[test]
fn handle_ui_changed_no_real_change() {
    let mut ui = MemoryUi::new();
    let mut vcs = MemoryVcs::new();
    let mut ci = MemoryCi::new();
    let mut db = MemoryDb::new();
    db.put_running(PipelineId(0), RunningEntry{
        pull_commit: MemoryCommit::A,
        merge_commit: Some(MemoryCommit::B),
        pr: MemoryPr::A,
        message: "MSG!".to_owned(),
        canceled: false,
    });
    handle_event(
        &mut ui,
        &mut vcs,
        &mut ci,
        &mut db,
        Event::UiEvent(ui::Event::Changed(
            PipelineId(0),
            MemoryPr::A,
            MemoryCommit::A,
        ))
    );
    assert_eq!(db.running.unwrap(), RunningEntry{
        pull_commit: MemoryCommit::A,
        merge_commit: Some(MemoryCommit::B),
        pr: MemoryPr::A,
        canceled: false,
        message: "MSG!".to_owned(),
    });
}

#[test]
fn handle_ui_changed_cancel_queue() {
    let mut ui = MemoryUi::new();
    let mut vcs = MemoryVcs::new();
    let mut ci = MemoryCi::new();
    let mut db = MemoryDb::new();
    db.put_running(PipelineId(0), RunningEntry{
        pull_commit: MemoryCommit::A,
        merge_commit: Some(MemoryCommit::B),
        pr: MemoryPr::A,
        message: "MSG!".to_owned(),
        canceled: false,
    });
    db.push_queue(PipelineId(0), QueueEntry{
        commit: MemoryCommit::C,
        pr: MemoryPr::B,
        message: "MSG!".to_owned(),
    });
    handle_event(
        &mut ui,
        &mut vcs,
        &mut ci,
        &mut db,
        Event::UiEvent(ui::Event::Changed(
            PipelineId(0),
            MemoryPr::B,
            MemoryCommit::D,
        ))
    );
    assert_eq!(db.running.unwrap(), RunningEntry{
        pull_commit: MemoryCommit::A,
        merge_commit: Some(MemoryCommit::B),
        pr: MemoryPr::A,
        canceled: false,
        message: "MSG!".to_owned(),
    });
    assert!(db.queue.is_empty());
}

#[test]
fn handle_ui_changed_no_real_change_queue() {
    let mut ui = MemoryUi::new();
    let mut vcs = MemoryVcs::new();
    let mut ci = MemoryCi::new();
    let mut db = MemoryDb::new();
    db.put_running(PipelineId(0), RunningEntry{
        pull_commit: MemoryCommit::A,
        merge_commit: Some(MemoryCommit::B),
        pr: MemoryPr::A,
        message: "MSG!".to_owned(),
        canceled: false,
    });
    db.push_queue(PipelineId(0), QueueEntry{
        commit: MemoryCommit::C,
        pr: MemoryPr::B,
        message: "MSG!".to_owned(),
    });
    handle_event(
        &mut ui,
        &mut vcs,
        &mut ci,
        &mut db,
        Event::UiEvent(ui::Event::Changed(
            PipelineId(0),
            MemoryPr::B,
            MemoryCommit::C,
        ))
    );
    assert_eq!(db.running.unwrap(), RunningEntry{
        pull_commit: MemoryCommit::A,
        merge_commit: Some(MemoryCommit::B),
        pr: MemoryPr::A,
        canceled: false,
        message: "MSG!".to_owned(),
    });
    assert_eq!(db.queue[0], QueueEntry{
        commit: MemoryCommit::C,
        pr: MemoryPr::B,
        message: "MSG!".to_owned(),
    });
}

#[test]
fn handle_ui_closed() {
    let mut ui = MemoryUi::new();
    let mut vcs = MemoryVcs::new();
    let mut ci = MemoryCi::new();
    let mut db = MemoryDb::new();
    db.put_running(PipelineId(0), RunningEntry{
        pull_commit: MemoryCommit::A,
        merge_commit: Some(MemoryCommit::B),
        pr: MemoryPr::A,
        message: "MSG!".to_owned(),
        canceled: false,
    });
    handle_event(
        &mut ui,
        &mut vcs,
        &mut ci,
        &mut db,
        Event::UiEvent(ui::Event::Closed(
            PipelineId(0),
            MemoryPr::A
        ))
    );
    assert_eq!(db.running.unwrap(), RunningEntry{
        pull_commit: MemoryCommit::A,
        merge_commit: Some(MemoryCommit::B),
        pr: MemoryPr::A,
        canceled: true,
        message: "MSG!".to_owned(),
    });
}

#[test]
fn handle_runthrough() {
    let mut ui = MemoryUi::new();
    let mut vcs = MemoryVcs::new();
    let mut ci = MemoryCi::new();
    let mut db = MemoryDb::new();
    handle_event(
        &mut ui,
        &mut vcs,
        &mut ci,
        &mut db,
        Event::UiEvent(ui::Event::Approved(
            PipelineId(0),
            MemoryPr::A,
            Some(MemoryCommit::A),
            "Message!".to_owned(),
        ))
    );
    assert_eq!(vcs.borrow().staging, Some(MemoryCommit::A));
    assert!(vcs.borrow().master.is_none());
    assert!(ci.borrow().build.is_none());
    assert!(ui.borrow().results.is_empty());
    vcs.borrow_mut().staging = Some(MemoryCommit::B);
    handle_event(
        &mut ui,
        &mut vcs,
        &mut ci,
        &mut db,
        Event::VcsEvent(vcs::Event::MergedToStaging(
            PipelineId(0),
            MemoryCommit::A,
            MemoryCommit::B,
        ))
    );
    assert_eq!(db.running, Some(RunningEntry{
        pull_commit: MemoryCommit::A,
        merge_commit: Some(MemoryCommit::B),
        pr: MemoryPr::A,
        canceled: false,
        message: "Message!".to_owned(),
    }));
    assert!(db.queue.is_empty());
    assert_eq!(vcs.borrow().staging, Some(MemoryCommit::B));
    assert!(vcs.borrow().master.is_none());
    assert_eq!(ci.borrow().build, Some(MemoryCommit::B));
    assert_eq!(ui.borrow().results, vec![
        (MemoryPr::A, ui::Status::InProgress),
    ]);
    handle_event(
        &mut ui,
        &mut vcs,
        &mut ci,
        &mut db,
        Event::CiEvent(ci::Event::BuildSucceeded(
            PipelineId(0),
            MemoryCommit::B
        ))
    );
    assert_eq!(vcs.borrow().staging, Some(MemoryCommit::B));
    assert_eq!(vcs.borrow().master, Some(MemoryCommit::B));
    assert_eq!(ui.borrow().results, vec![
        (MemoryPr::A, ui::Status::InProgress),
    ]);
    handle_event(
        &mut ui,
        &mut vcs,
        &mut ci,
        &mut db,
        Event::VcsEvent(vcs::Event::MovedToMaster(
            PipelineId(0),
            MemoryCommit::B
        ))
    );
    assert_eq!(vcs.borrow().staging, Some(MemoryCommit::B));
    assert_eq!(vcs.borrow().master, Some(MemoryCommit::B));
    assert_eq!(ui.borrow().results, vec![
        (MemoryPr::A, ui::Status::InProgress),
        (MemoryPr::A, ui::Status::Success),
    ]);
}

#[test]
fn handle_runthrough_next_commit() {
    let mut ui = MemoryUi::new();
    let mut vcs = MemoryVcs::new();
    let mut ci = MemoryCi::new();
    let mut db = MemoryDb::new();
    // Add a first item to the queue. This one should be built first.
    handle_event(
        &mut ui,
        &mut vcs,
        &mut ci,
        &mut db,
        Event::UiEvent(ui::Event::Approved(
            PipelineId(0),
            MemoryPr::A,
            Some(MemoryCommit::A),
            "MSG!".to_owned(),
        ))
    );
    assert_eq!(db.running, Some(RunningEntry{
        pull_commit: MemoryCommit::A,
        merge_commit: None,
        pr: MemoryPr::A,
        canceled: false,
        message: "MSG!".to_owned(),
    }));
    assert_eq!(vcs.borrow().staging, Some(MemoryCommit::A));
    assert!(vcs.borrow().master.is_none());
    assert!(ci.borrow().build.is_none());
    assert!(ui.borrow().results.is_empty());
    // Add a second item to the queue. Since the first is not done merging
    // into the staging area, the build state should not have changed.
    handle_event(
        &mut ui,
        &mut vcs,
        &mut ci,
        &mut db,
        Event::UiEvent(ui::Event::Approved(
            PipelineId(0),
            MemoryPr::C,
            Some(MemoryCommit::C),
            "Message!".to_owned(),
        ))
    );
    assert_eq!(db.running, Some(RunningEntry{
        pull_commit: MemoryCommit::A,
        merge_commit: None,
        pr: MemoryPr::A,
        canceled: false,
        message: "MSG!".to_owned(),
    }));
    assert_eq!(db.queue.len(), 1);
    // The first is now done merging. It should now be sent to the CI.
    vcs.borrow_mut().staging = Some(MemoryCommit::B);
    handle_event(
        &mut ui,
        &mut vcs,
        &mut ci,
        &mut db,
        Event::VcsEvent(vcs::Event::MergedToStaging(
            PipelineId(0),
            MemoryCommit::A,
            MemoryCommit::B,
        ))
    );
    assert_eq!(db.running, Some(RunningEntry{
        pull_commit: MemoryCommit::A,
        merge_commit: Some(MemoryCommit::B),
        pr: MemoryPr::A,
        canceled: false,
        message: "MSG!".to_owned(),
    }));
    assert_eq!(vcs.borrow().staging, Some(MemoryCommit::B));
    assert!(vcs.borrow().master.is_none());
    assert_eq!(ci.borrow().build, Some(MemoryCommit::B));
    assert_eq!(ui.borrow().results, vec![
        (MemoryPr::A, ui::Status::InProgress),
    ]);
    // The CI successfully built it. It should now be moved to master.
    handle_event(
        &mut ui,
        &mut vcs,
        &mut ci,
        &mut db,
        Event::CiEvent(ci::Event::BuildSucceeded(
            PipelineId(0),
            MemoryCommit::B
        ))
    );
    assert_eq!(vcs.borrow().staging, Some(MemoryCommit::B));
    assert_eq!(vcs.borrow().master, Some(MemoryCommit::B));
    assert_eq!(ui.borrow().results, vec![
        (MemoryPr::A, ui::Status::InProgress),
    ]);
    // It has been successfully moved to master. The next build should
    // start, and this one should be reported complete.
    handle_event(
        &mut ui,
        &mut vcs,
        &mut ci,
        &mut db,
        Event::VcsEvent(vcs::Event::MovedToMaster(
            PipelineId(0),
            MemoryCommit::B
        ))
    );
    assert_eq!(vcs.borrow().staging, Some(MemoryCommit::C));
    assert_eq!(vcs.borrow().master, Some(MemoryCommit::B));
    assert_eq!(ui.borrow().results, vec![
        (MemoryPr::A, ui::Status::InProgress),
        (MemoryPr::A, ui::Status::Success),
    ]);
    // The second one is now merged into staging; let's start building.
    vcs.borrow_mut().staging = Some(MemoryCommit::D);
    handle_event(
        &mut ui,
        &mut vcs,
        &mut ci,
        &mut db,
        Event::VcsEvent(vcs::Event::MergedToStaging(
            PipelineId(0),
            MemoryCommit::C,
            MemoryCommit::D,
        ))
    );
    assert_eq!(ci.borrow().build, Some(MemoryCommit::D));
    assert_eq!(ui.borrow().results, vec![
        (MemoryPr::A, ui::Status::InProgress),
        (MemoryPr::A, ui::Status::Success),
        (MemoryPr::C, ui::Status::InProgress),
    ]);
}

} // mod test