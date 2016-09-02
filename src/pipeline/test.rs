// This file is released under the same terms as Rust itself.

use super::{Ci, Vcs, Ui};
use ci;
use db::{Db, PendingEntry, QueueEntry, RunningEntry};
use hyper::Url;
use hyper::client::IntoUrl;
use pipeline::{Event, Pipeline, PipelineId};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::error::Error;
use std::fmt::{self, Debug, Display};
use std::marker::PhantomData;
use std::mem;
use std::str::FromStr;
use ui::{self, Pr};
use vcs::{self, Commit};
use void::Void;

struct MemoryDb<P: Pr> {
    queue: VecDeque<QueueEntry<P>>,
    running: Option<RunningEntry<P>>,
    pending: Vec<PendingEntry<P>>,
}

impl<P: Pr> MemoryDb<P> {
    fn new() -> Self {
        MemoryDb{
            queue: VecDeque::new(),
            running: None,
            pending: Vec::new(),
        }
    }
}

impl<P: Pr> Db<P> for MemoryDb<P> {
    fn push_queue(
        &mut self,
        _: PipelineId,
        entry: QueueEntry<P>,
    ) -> Result<(), Box<Error + Send + Sync>> {
        self.queue.push_back(entry);
        Ok(())
    }
    fn pop_queue(
        &mut self,
        _: PipelineId
    ) -> Result<Option<QueueEntry<P>>, Box<Error + Send + Sync>> {
        Ok(self.queue.pop_front())
    }
    fn list_queue(
        &mut self,
        _: PipelineId,
    ) -> Result<Vec<QueueEntry<P>>, Box<Error + Send + Sync>> {
        unimplemented!()
    }
    fn put_running(
        &mut self,
        _: PipelineId,
        entry: RunningEntry<P>,
    ) -> Result<(), Box<Error + Send + Sync>> {
        self.running = Some(entry);
        Ok(())
    }
    fn take_running(
        &mut self,
        _: PipelineId,
    ) -> Result<Option<RunningEntry<P>>, Box<Error + Send + Sync>> {
        Ok(mem::replace(&mut self.running, None))
    }
    fn peek_running(
        &mut self,
        _: PipelineId,
    ) -> Result<Option<RunningEntry<P>>, Box<Error + Send + Sync>> {
        Ok(self.running.clone())
    }
    fn add_pending(
        &mut self,
        _: PipelineId,
        entry: PendingEntry<P>,
    ) -> Result<(), Box<Error + Send + Sync>>{
        let mut replaced = false;
        for entry2 in self.pending.iter_mut() {
            if entry2.pr == entry.pr {
                mem::replace(entry2, entry.clone());
                replaced = true;
                break;
            }
        }
        if !replaced {
            self.pending.push(entry);
        }
        Ok(())
    }
    fn peek_pending_by_pr(
        &mut self,
        _: PipelineId,
        pr: &P,
    ) -> Result<Option<PendingEntry<P>>, Box<Error + Send + Sync>> {
        for entry in &self.pending {
            if entry.pr == *pr {
                return Ok(Some(entry.clone()));
            }
        }
        Ok(None)
    }
    fn take_pending_by_pr(
        &mut self,
        _: PipelineId,
        pr: &P,
    ) -> Result<Option<PendingEntry<P>>, Box<Error + Send + Sync>> {
        let mut entry_i = None;
        for (i, entry) in self.pending.iter().enumerate() {
            if entry.pr == *pr {
                entry_i = Some(i);
                break;
            }
        }
        Ok(entry_i.map(|entry_i| self.pending.remove(entry_i)))
    }
    fn list_pending(
        &mut self,
        _: PipelineId,
    ) -> Result<Vec<PendingEntry<P>>, Box<Error + Send + Sync>> {
        unimplemented!()
    }
    fn cancel_by_pr(
        &mut self,
        _: PipelineId,
        pr: &P,
    ) -> Result<(), Box<Error + Send + Sync>> {
        let queue = mem::replace(&mut self.queue, VecDeque::new());
        let filtered = queue.into_iter().filter(|entry| entry.pr != *pr);
        self.queue.extend(filtered);
        if let Some(ref mut running) = self.running {
            if running.pr == *pr {
                running.canceled = true;
            }
        }
        Ok(())
    }
    fn cancel_by_pr_different_commit(
        &mut self,
        _: PipelineId,
        pr: &P,
        commit: &P::C
    ) -> Result<bool, Box<Error + Send + Sync>> {
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
        Ok(canceled)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(unused)]
enum MemoryCommit {
    A, B, C, D, E, F, G, H, I, J, K, L, M,
    N, O, P, Q, R, S, T, U, V, W, X, Y, Z
}
impl Commit for MemoryCommit {
    type Remote = String;
}
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
    type C = MemoryCommit;
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
    results: Vec<(MemoryPr, ui::Status<MemoryPr>)>,
}
impl MemoryUi {
    fn new() -> RefCell<MemoryUi> {
        RefCell::new(MemoryUi{
            results: Vec::new(),
        })
    }
}
impl Ui<MemoryPr> for RefCell<MemoryUi> {
    fn send_result(
        &self,
        _: PipelineId,
        pr: MemoryPr,
        status: ui::Status<MemoryPr>,
    ) {
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
    db: &mut MemoryDb<MemoryPr>,
    event: Event<MemoryPr>,
) {
    Pipeline{
        _pr: PhantomData,
        ui: ui,
        vcs: vcs,
        ci: ci,
        id: PipelineId(0),
    }.handle_event(db, event).unwrap();
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
            "".to_owned(),
            Url::parse("http://www.com/").unwrap(),
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
fn handle_add_to_queue_by_pending_changed() {
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
            "".to_owned(),
            Url::parse("http://www.com/").unwrap(),
        )),
    );
    handle_event(
        &mut ui,
        &mut vcs,
        &mut ci,
        &mut db,
        Event::UiEvent(ui::Event::Changed(
            PipelineId(0),
            MemoryPr::A,
            MemoryCommit::B,
            "".to_owned(),
            Url::parse("http://www.com/").unwrap(),
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
    assert_eq!(db.running.unwrap().pull_commit, MemoryCommit::B);
    assert!(db.queue.is_empty());
    assert_eq!(vcs.borrow().staging.unwrap(), MemoryCommit::B);
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
        built: false,
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
        vec![(MemoryPr::A, ui::Status::Unmergeable(MemoryCommit::A))]
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
        built: false,
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
        built: false,
    });
    assert!(db.queue.is_empty());
    assert!(ci.borrow().build.is_none());
    assert!(vcs.borrow().master.is_none());
    assert_eq!(vcs.borrow().staging.unwrap(), MemoryCommit::C);
    assert_eq!(
        ui.borrow().results,
        vec![(MemoryPr::A, ui::Status::Unmergeable(MemoryCommit::A))]
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
        built: false,
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
        built: false,
    });
    assert!(db.queue.is_empty());
    assert_eq!(ci.borrow().build.unwrap(), MemoryCommit::B);
    assert!(vcs.borrow().master.is_none());
    assert_eq!(vcs.borrow().staging.unwrap(), MemoryCommit::B);
    assert_eq!(
        ui.borrow().results,
        vec![(MemoryPr::A, ui::Status::StartingBuild(
            MemoryCommit::A,
            MemoryCommit::B,
        ))]
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
        built: false,
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
            None,
        ))
    );
    assert!(db.running.is_none());
    assert!(db.queue.is_empty());
    assert!(vcs.borrow().master.is_none());
    assert_eq!(vcs.borrow().staging.unwrap(), MemoryCommit::B);
    assert_eq!(
        ui.borrow().results,
        vec![(MemoryPr::A, ui::Status::Failure(
            MemoryCommit::A,
            MemoryCommit::B,
            None,
        ))]
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
        built: false,
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
            None,
        ))
    );
    assert_eq!(db.running.unwrap(), RunningEntry{
        pull_commit: MemoryCommit::C,
        merge_commit: None,
        pr: MemoryPr::B,
        message: "M!".to_owned(),
        canceled: false,
        built: false,
    });
    assert!(db.queue.is_empty());
    assert!(vcs.borrow().master.is_none());
    assert_eq!(vcs.borrow().staging.unwrap(), MemoryCommit::C);
    assert_eq!(
        ui.borrow().results,
        vec![(MemoryPr::A, ui::Status::Failure(
            MemoryCommit::A,
            MemoryCommit::B,
            None,
        ))]
    );
}

#[test]
fn handle_ci_started_notify_user() {
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
        built: false,
    });
    vcs.borrow_mut().staging = Some(MemoryCommit::B);
    handle_event(
        &mut ui,
        &mut vcs,
        &mut ci,
        &mut db,
        Event::CiEvent(ci::Event::BuildStarted(
            PipelineId(0),
            MemoryCommit::B,
            Some("http://example.com/".into_url().expect("this to be valid")),
        ))
    );
    assert!(db.running.is_some());
    assert!(db.queue.is_empty());
    assert_eq!(
        ui.borrow().results,
        vec![(MemoryPr::A, ui::Status::Testing(
            MemoryCommit::A,
            MemoryCommit::B,
            Some("http://example.com/".into_url().expect("this to be valid")),
        ))]
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
        built: false,
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
            None,
        ))
    );
    assert!(db.running.is_some());
    assert!(db.queue.is_empty());
    assert_eq!(vcs.borrow().master.unwrap(), MemoryCommit::B);
    assert_eq!(
        ui.borrow().results,
        vec![
            (MemoryPr::A, ui::Status::Success(
                MemoryCommit::A,
                MemoryCommit::B,
                None,
            ))
        ]
    );
}

#[test]
fn handle_ci_double_succeeded_move_to_master() {
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
        built: false,
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
            None,
        ))
    );
    assert!(db.running.is_some());
    assert!(db.queue.is_empty());
    assert_eq!(vcs.borrow().master.unwrap(), MemoryCommit::B);
    assert_eq!(
        ui.borrow().results,
        vec![
            (MemoryPr::A, ui::Status::Success(
                MemoryCommit::A,
                MemoryCommit::B,
                None,
            ))
        ]
    );
    handle_event(
        &mut ui,
        &mut vcs,
        &mut ci,
        &mut db,
        Event::CiEvent(ci::Event::BuildSucceeded(
            PipelineId(0),
            MemoryCommit::B,
            None,
        ))
    );
    assert!(db.running.is_some());
    assert!(db.queue.is_empty());
    assert_eq!(vcs.borrow().master.unwrap(), MemoryCommit::B);
    assert_eq!(
        ui.borrow().results,
        vec![
            (MemoryPr::A, ui::Status::Success(
                MemoryCommit::A,
                MemoryCommit::B,
                None,
            ))
        ]
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
        built: true,
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
        vec![(MemoryPr::A, ui::Status::Unmoveable(
            MemoryCommit::A,
            MemoryCommit::B,
        ))]
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
        built: true,
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
        built: false,
    });
    assert!(db.queue.is_empty());
    assert!(vcs.borrow().master.is_none());
    assert_eq!(vcs.borrow().staging.unwrap(), MemoryCommit::C);
    assert_eq!(
        ui.borrow().results,
        vec![(MemoryPr::A, ui::Status::Unmoveable(
            MemoryCommit::A,
            MemoryCommit::B,
        ))]
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
        built: true,
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
        vec![(MemoryPr::A, ui::Status::Completed(
            MemoryCommit::A,
            MemoryCommit::B,
        ))]
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
        built: true,
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
        built: false,
    });
    assert!(db.queue.is_empty());
    assert!(vcs.borrow().master.is_none());
    assert_eq!(vcs.borrow().staging.unwrap(), MemoryCommit::C);
    assert_eq!(
        ui.borrow().results,
        vec![
            (MemoryPr::A, ui::Status::Completed(
                MemoryCommit::A,
                MemoryCommit::B,
            ))
        ]
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
        built: false,
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
        built: false,
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
        built: false,
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
            "".to_owned(),
            Url::parse("http://www.com/").unwrap(),
        ))
    );
    assert_eq!(db.running.unwrap(), RunningEntry{
        pull_commit: MemoryCommit::A,
        merge_commit: Some(MemoryCommit::B),
        pr: MemoryPr::A,
        canceled: true,
        built: false,
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
        built: false,
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
            "".to_owned(),
            Url::parse("http://www.com/").unwrap(),
        ))
    );
    assert_eq!(db.running.unwrap(), RunningEntry{
        pull_commit: MemoryCommit::A,
        merge_commit: Some(MemoryCommit::B),
        pr: MemoryPr::A,
        canceled: false,
        built: false,
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
        built: false,
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
            "".to_owned(),
            Url::parse("http://www.com/").unwrap(),
        ))
    );
    assert_eq!(db.running.unwrap(), RunningEntry{
        pull_commit: MemoryCommit::A,
        merge_commit: Some(MemoryCommit::B),
        pr: MemoryPr::A,
        canceled: false,
        built: false,
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
        built: false,
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
            "".to_owned(),
            Url::parse("http://www.com/").unwrap(),
        ))
    );
    assert_eq!(db.running.unwrap(), RunningEntry{
        pull_commit: MemoryCommit::A,
        merge_commit: Some(MemoryCommit::B),
        pr: MemoryPr::A,
        canceled: false,
        built: false,
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
        built: false,
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
        built: false,
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
    assert_eq!(ui.borrow().results, vec![
        (MemoryPr::A, ui::Status::Approved(
            MemoryCommit::A,
        )),
    ]);
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
        built: false,
        message: "Message!".to_owned(),
    }));
    assert!(db.queue.is_empty());
    assert_eq!(vcs.borrow().staging, Some(MemoryCommit::B));
    assert!(vcs.borrow().master.is_none());
    assert_eq!(ci.borrow().build, Some(MemoryCommit::B));
    assert_eq!(ui.borrow().results, vec![
        (MemoryPr::A, ui::Status::Approved(
            MemoryCommit::A,
        )),
        (MemoryPr::A, ui::Status::StartingBuild(
            MemoryCommit::A,
            MemoryCommit::B,
        )),
    ]);
    handle_event(
        &mut ui,
        &mut vcs,
        &mut ci,
        &mut db,
        Event::CiEvent(ci::Event::BuildSucceeded(
            PipelineId(0),
            MemoryCommit::B,
            None,
        ))
    );
    assert_eq!(vcs.borrow().staging, Some(MemoryCommit::B));
    assert_eq!(vcs.borrow().master, Some(MemoryCommit::B));
    assert_eq!(ui.borrow().results, vec![
        (MemoryPr::A, ui::Status::Approved(
            MemoryCommit::A,
        )),
        (MemoryPr::A, ui::Status::StartingBuild(
            MemoryCommit::A,
            MemoryCommit::B,
        )),
        (MemoryPr::A, ui::Status::Success(
            MemoryCommit::A,
            MemoryCommit::B,
            None,
        )),
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
        (MemoryPr::A, ui::Status::Approved(
            MemoryCommit::A,
        )),
        (MemoryPr::A, ui::Status::StartingBuild(
            MemoryCommit::A,
            MemoryCommit::B,
        )),
        (MemoryPr::A, ui::Status::Success(
            MemoryCommit::A,
            MemoryCommit::B,
            None,
        )),
        (MemoryPr::A, ui::Status::Completed(
            MemoryCommit::A,
            MemoryCommit::B,
        )),
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
        built: false,
        message: "MSG!".to_owned(),
    }));
    assert_eq!(vcs.borrow().staging, Some(MemoryCommit::A));
    assert!(vcs.borrow().master.is_none());
    assert!(ci.borrow().build.is_none());
    assert_eq!(ui.borrow().results, vec![
        (MemoryPr::A, ui::Status::Approved(
            MemoryCommit::A,
        )),
    ]);
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
        built: false,
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
        built: false,
        message: "MSG!".to_owned(),
    }));
    assert_eq!(vcs.borrow().staging, Some(MemoryCommit::B));
    assert!(vcs.borrow().master.is_none());
    assert_eq!(ci.borrow().build, Some(MemoryCommit::B));
    assert_eq!(ui.borrow().results, vec![
        (MemoryPr::A, ui::Status::Approved(
            MemoryCommit::A,
        )),
        (MemoryPr::C, ui::Status::Approved(
            MemoryCommit::C,
        )),
        (MemoryPr::A, ui::Status::StartingBuild(
            MemoryCommit::A,
            MemoryCommit::B,
        )),
    ]);
    // The CI successfully built it. It should now be moved to master.
    handle_event(
        &mut ui,
        &mut vcs,
        &mut ci,
        &mut db,
        Event::CiEvent(ci::Event::BuildSucceeded(
            PipelineId(0),
            MemoryCommit::B,
            None,
        ))
    );
    assert_eq!(vcs.borrow().staging, Some(MemoryCommit::B));
    assert_eq!(vcs.borrow().master, Some(MemoryCommit::B));
    assert_eq!(ui.borrow().results, vec![
        (MemoryPr::A, ui::Status::Approved(
            MemoryCommit::A,
        )),
        (MemoryPr::C, ui::Status::Approved(
            MemoryCommit::C,
        )),
        (MemoryPr::A, ui::Status::StartingBuild(
            MemoryCommit::A,
            MemoryCommit::B,
        )),
        (MemoryPr::A, ui::Status::Success(
            MemoryCommit::A,
            MemoryCommit::B,
            None,
        )),
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
        (MemoryPr::A, ui::Status::Approved(
            MemoryCommit::A,
        )),
        (MemoryPr::C, ui::Status::Approved(
            MemoryCommit::C,
        )),
        (MemoryPr::A, ui::Status::StartingBuild(
            MemoryCommit::A,
            MemoryCommit::B,
        )),
        (MemoryPr::A, ui::Status::Success(
            MemoryCommit::A,
            MemoryCommit::B,
            None,
        )),
        (MemoryPr::A, ui::Status::Completed(
            MemoryCommit::A,
            MemoryCommit::B,
        )),
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
        (MemoryPr::A, ui::Status::Approved(
            MemoryCommit::A,
        )),
        (MemoryPr::C, ui::Status::Approved(
            MemoryCommit::C,
        )),
        (MemoryPr::A, ui::Status::StartingBuild(
            MemoryCommit::A,
            MemoryCommit::B,
        )),
        (MemoryPr::A, ui::Status::Success(
            MemoryCommit::A,
            MemoryCommit::B,
            None,
        )),
        (MemoryPr::A, ui::Status::Completed(
            MemoryCommit::A,
            MemoryCommit::B,
        )),
        (MemoryPr::C, ui::Status::StartingBuild(
            MemoryCommit::C,
            MemoryCommit::D,
        )),
    ]);
}
