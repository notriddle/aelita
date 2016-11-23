// This file is released under the same terms as Rust itself.

use super::{Ci, Vcs, Ui};
use ci::{self, CiId};
use db::{CiState, Db, PendingEntry, QueueEntry, RunningEntry};
use hyper::Url;
use hyper::client::IntoUrl;
use pipeline::{Event, Pipeline, PipelineId};
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::error::Error;
use std::mem;
use ui::{self, Pr};
use vcs::{self, Commit, Remote};

struct MemoryDb {
    queue: VecDeque<QueueEntry>,
    running: Option<RunningEntry>,
    pending: Vec<PendingEntry>,
    cis: HashMap<CiId, (CiState, Commit)>,
}

impl MemoryDb {
    fn new() -> Self {
        MemoryDb{
            queue: VecDeque::new(),
            running: None,
            pending: Vec::new(),
            cis: HashMap::new(),
        }
    }
}

impl Db for MemoryDb {
    fn push_queue(
        &mut self,
        _: PipelineId,
        entry: QueueEntry,
    ) -> Result<(), Box<Error + Send + Sync>> {
        self.queue.push_back(entry);
        Ok(())
    }
    fn pop_queue(
        &mut self,
        _: PipelineId
    ) -> Result<Option<QueueEntry>, Box<Error + Send + Sync>> {
        Ok(self.queue.pop_front())
    }
    fn list_queue(
        &mut self,
        _: PipelineId,
    ) -> Result<Vec<QueueEntry>, Box<Error + Send + Sync>> {
        unimplemented!()
    }
    fn put_running(
        &mut self,
        _: PipelineId,
        entry: RunningEntry,
    ) -> Result<(), Box<Error + Send + Sync>> {
        self.running = Some(entry);
        Ok(())
    }
    fn take_running(
        &mut self,
        _: PipelineId,
    ) -> Result<Option<RunningEntry>, Box<Error + Send + Sync>> {
        Ok(mem::replace(&mut self.running, None))
    }
    fn peek_running(
        &mut self,
        _: PipelineId,
    ) -> Result<Option<RunningEntry>, Box<Error + Send + Sync>> {
        Ok(self.running.clone())
    }
    fn add_pending(
        &mut self,
        _: PipelineId,
        entry: PendingEntry,
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
        pr: &Pr,
    ) -> Result<Option<PendingEntry>, Box<Error + Send + Sync>> {
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
        pr: &Pr,
    ) -> Result<Option<PendingEntry>, Box<Error + Send + Sync>> {
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
    ) -> Result<Vec<PendingEntry>, Box<Error + Send + Sync>> {
        unimplemented!()
    }
    fn cancel_by_pr(
        &mut self,
        _: PipelineId,
        pr: &Pr,
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
        pr: &Pr,
        commit: &Commit,
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
    fn set_ci_state(
        &mut self,
        ci_id: CiId,
        ci_state: CiState,
        commit: &Commit,
    ) -> Result<(), Box<Error + Send + Sync>> {
        self.cis.insert(ci_id, (ci_state, commit.clone()));
        Ok(())
    }
    fn clear_ci_state(
        &mut self,
        ci_id: CiId,
    ) -> Result<(), Box<Error + Send + Sync>> {
        self.cis.remove(&ci_id);
        Ok(())
    }
    fn get_ci_state(
        &mut self,
        ci_id: CiId,
    ) -> Result<Option<(CiState, Commit)>, Box<Error + Send + Sync>> {
        Ok(self.cis.get(&ci_id).cloned())
    }
}

struct MemoryUi {
    results: Vec<(Pr, ui::Status)>,
}
impl MemoryUi {
    fn new() -> RefCell<MemoryUi> {
        RefCell::new(MemoryUi{
            results: Vec::new(),
        })
    }
}
impl Ui for RefCell<MemoryUi> {
    fn send_result(
        &self,
        _: PipelineId,
        pr: Pr,
        status: ui::Status,
    ) {
        self.borrow_mut().results.push((pr, status));
    }
}

struct MemoryVcs {
    staging: Option<Commit>,
    master: Option<Commit>,
}
impl MemoryVcs {
    fn new() -> RefCell<MemoryVcs> {
        RefCell::new(MemoryVcs{
            staging: None,
            master: None,
        })
    }
}
impl Vcs for RefCell<MemoryVcs> {
    fn merge_to_staging(
        &self,
        _: PipelineId,
        pull_commit: Commit,
        _message: String,
        _remote: Remote,
    ) {
        self.borrow_mut().staging = Some(pull_commit)
    }
    fn move_staging_to_master(&self, _: PipelineId, commit: Commit) {
        self.borrow_mut().master = Some(commit)
    }
}

struct MemoryCi {
    build: Option<Commit>,
}
impl MemoryCi {
    fn new() -> RefCell<MemoryCi> {
        RefCell::new(MemoryCi{
            build: None,
        })
    }
}
impl Ci for RefCell<MemoryCi> {
    fn start_build(&self, _: CiId, commit: Commit) {
        self.borrow_mut().build = Some(commit);
    }
}


fn handle_event(
    ui: &mut RefCell<MemoryUi>,
    vcs: &mut RefCell<MemoryVcs>,
    ci: &mut RefCell<MemoryCi>,
    db: &mut MemoryDb,
    event: Event,
) {
    Pipeline{
        ui: ui,
        vcs: vcs,
        ci: vec![(CiId(1), ci)],
        id: PipelineId(0),
    }.handle_event(db, event).unwrap();
}

fn handle_event_2_ci(
    ui: &mut RefCell<MemoryUi>,
    vcs: &mut RefCell<MemoryVcs>,
    ci1: &mut RefCell<MemoryCi>,
    ci2: &mut RefCell<MemoryCi>,
    db: &mut MemoryDb,
    event: Event,
) {
    Pipeline{
        ui: ui,
        vcs: vcs,
        ci: vec![(CiId(1), ci1), (CiId(2), ci2)],
        id: PipelineId(0),
    }.handle_event(db, event).unwrap();
}


fn memory_commit_a() -> Commit {
    Commit::from("A".to_owned())
}
fn memory_commit_b() -> Commit {
    Commit::from("B".to_owned())
}
fn memory_commit_c() -> Commit {
    Commit::from("C".to_owned())
}
fn memory_commit_d() -> Commit {
    Commit::from("D".to_owned())
}
fn memory_pr_a() -> Pr {
    Pr::from("A".to_owned())
}
fn memory_pr_b() -> Pr {
    Pr::from("B".to_owned())
}
fn memory_pr_c() -> Pr {
    Pr::from("C".to_owned())
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
            memory_pr_a(),
            Some(memory_commit_a()),
            "Message!".to_owned(),
        )),
    );
    assert_eq!(db.running.unwrap().pull_commit, memory_commit_a());
    assert!(db.queue.is_empty());
    assert_eq!(vcs.borrow().staging.as_ref().unwrap(), &memory_commit_a());
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
            memory_pr_a(),
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
            memory_pr_a(),
            memory_commit_a(),
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
            memory_pr_a(),
            None,
            "Message!".to_owned(),
        )),
    );
    assert_eq!(db.running.unwrap().pull_commit, memory_commit_a());
    assert!(db.queue.is_empty());
    assert_eq!(vcs.borrow().staging.as_ref().unwrap(), &memory_commit_a());
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
            memory_pr_a(),
            memory_commit_a(),
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
            memory_pr_a(),
            memory_commit_b(),
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
            memory_pr_a(),
            None,
            "Message!".to_owned(),
        )),
    );
    assert_eq!(db.running.unwrap().pull_commit, memory_commit_b());
    assert!(db.queue.is_empty());
    assert_eq!(vcs.borrow().staging.as_ref().unwrap(), &memory_commit_b());
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
            memory_pr_a(),
            Some(memory_commit_a()),
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
            memory_pr_b(),
            Some(memory_commit_b()),
            "Message!".to_owned(),
        ))
    );
    assert!(!db.running.clone().unwrap().canceled);
    assert_eq!(db.running.unwrap().pull_commit, memory_commit_a());
    assert_eq!(db.queue.front().unwrap().commit, memory_commit_b());
    assert_eq!(vcs.borrow().staging.as_ref().unwrap(), &memory_commit_a());
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
            memory_pr_a(),
            Some(memory_commit_a()),
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
            memory_pr_a(),
            Some(memory_commit_b()),
            "Message!".to_owned(),
        ))
    );
    assert!(db.running.clone().unwrap().canceled);
    assert_eq!(db.running.unwrap().pull_commit, memory_commit_a());
    assert_eq!(db.queue.front().unwrap().commit, memory_commit_b());
    assert_eq!(vcs.borrow().staging.as_ref().unwrap(), &memory_commit_a());
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
            memory_pr_a(),
            Some(memory_commit_a()),
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
            memory_pr_a(),
            Some(memory_commit_b()),
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
            memory_pr_a(),
            Some(memory_commit_c()),
            "Message!".to_owned(),
        ))
    );
    assert!(db.running.clone().unwrap().canceled);
    assert_eq!(db.running.unwrap().pull_commit, memory_commit_a());
    assert_eq!(db.queue.front().unwrap().commit, memory_commit_c());
    assert_eq!(vcs.borrow().staging.as_ref().unwrap(), &memory_commit_a());
}

#[test]
fn handle_merge_failed_notify_user() {
    let mut ui = MemoryUi::new();
    let mut vcs = MemoryVcs::new();
    let mut ci = MemoryCi::new();
    let mut db = MemoryDb::new();
    db.put_running(PipelineId(0), RunningEntry{
        pull_commit: memory_commit_a(),
        merge_commit: None,
        pr: memory_pr_a(),
        message: "MSG!".to_owned(),
        canceled: false,
        built: false,
    }).unwrap();
    handle_event(
        &mut ui,
        &mut vcs,
        &mut ci,
        &mut db,
        Event::VcsEvent(vcs::Event::FailedMergeToStaging(
            PipelineId(0),
            memory_commit_a()
        ))
    );
    assert!(db.running.is_none());
    assert!(db.queue.is_empty());
    assert!(ci.borrow().build.is_none());
    assert!(vcs.borrow().master.is_none());
    assert_eq!(
        ui.borrow().results,
        vec![(memory_pr_a(), ui::Status::Unmergeable(memory_commit_a()))]
    );
}

#[test]
fn handle_merge_failed_notify_user_merge_next_commit() {
    let mut ui = MemoryUi::new();
    let mut vcs = MemoryVcs::new();
    let mut ci = MemoryCi::new();
    let mut db = MemoryDb::new();
    db.put_running(PipelineId(0), RunningEntry{
        pull_commit: memory_commit_a(),
        merge_commit: None,
        pr: memory_pr_a(),
        message: "MSG!".to_owned(),
        canceled: false,
        built: false,
    }).unwrap();
    db.push_queue(PipelineId(0), QueueEntry{
        commit: memory_commit_c(),
        pr: memory_pr_b(),
        message: "M!".to_owned(),
    }).unwrap();
    handle_event(
        &mut ui,
        &mut vcs,
        &mut ci,
        &mut db,
        Event::VcsEvent(vcs::Event::FailedMergeToStaging(
            PipelineId(0),
            memory_commit_a()
        ))
    );
    assert_eq!(db.running.unwrap(), RunningEntry{
        pull_commit: memory_commit_c(),
        merge_commit: None,
        pr: memory_pr_b(),
        message: "M!".to_owned(),
        canceled: false,
        built: false,
    });
    assert!(db.queue.is_empty());
    assert!(ci.borrow().build.is_none());
    assert!(vcs.borrow().master.is_none());
    assert_eq!(vcs.borrow().staging.as_ref().unwrap(), &memory_commit_c());
    assert_eq!(
        ui.borrow().results,
        vec![(memory_pr_a(), ui::Status::Unmergeable(memory_commit_a()))]
    );
}

#[test]
fn handle_merge_succeeded_notify_user_start_ci() {
    let mut ui = MemoryUi::new();
    let mut vcs = MemoryVcs::new();
    let mut ci = MemoryCi::new();
    let mut db = MemoryDb::new();
    db.put_running(PipelineId(0), RunningEntry{
        pull_commit: memory_commit_a(),
        merge_commit: None,
        pr: memory_pr_a(),
        message: "MSG!".to_owned(),
        canceled: false,
        built: false,
    }).unwrap();
    vcs.borrow_mut().staging = Some(memory_commit_b());
    handle_event(
        &mut ui,
        &mut vcs,
        &mut ci,
        &mut db,
        Event::VcsEvent(vcs::Event::MergedToStaging(
            PipelineId(0),
            memory_commit_a(),
            memory_commit_b()
        )),
    );
    assert_eq!(db.running.unwrap(), RunningEntry{
        pull_commit: memory_commit_a(),
        merge_commit: Some(memory_commit_b()),
        pr: memory_pr_a(),
        message: "MSG!".to_owned(),
        canceled: false,
        built: false,
    });
    assert!(db.queue.is_empty());
    assert_eq!(ci.borrow().build.as_ref().unwrap(), &memory_commit_b());
    assert!(vcs.borrow().master.is_none());
    assert_eq!(vcs.borrow().staging.as_ref().unwrap(), &memory_commit_b());
    assert_eq!(
        ui.borrow().results,
        vec![(memory_pr_a(), ui::Status::StartingBuild(
            memory_commit_a(),
            memory_commit_b(),
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
        pull_commit: memory_commit_a(),
        merge_commit: Some(memory_commit_b()),
        pr: memory_pr_a(),
        message: "MSG!".to_owned(),
        canceled: false,
        built: false,
    }).unwrap();
    vcs.borrow_mut().staging = Some(memory_commit_b());
    handle_event(
        &mut ui,
        &mut vcs,
        &mut ci,
        &mut db,
        Event::CiEvent(ci::Event::BuildFailed(
            CiId(1),
            memory_commit_b(),
            None,
        ))
    );
    assert!(db.running.is_none());
    assert!(db.queue.is_empty());
    assert!(vcs.borrow().master.is_none());
    assert_eq!(vcs.borrow().staging.as_ref().unwrap(), &memory_commit_b());
    assert_eq!(
        ui.borrow().results,
        vec![(memory_pr_a(), ui::Status::Failure(
            memory_commit_a(),
            memory_commit_b(),
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
        pull_commit: memory_commit_a(),
        merge_commit: Some(memory_commit_b()),
        pr: memory_pr_a(),
        message: "MSG!".to_owned(),
        canceled: false,
        built: false,
    }).unwrap();
    db.push_queue(PipelineId(0), QueueEntry{
        commit: memory_commit_c(),
        pr: memory_pr_b(),
        message: "M!".to_owned(),
    }).unwrap();
    vcs.borrow_mut().staging = Some(memory_commit_b());
    handle_event(
        &mut ui,
        &mut vcs,
        &mut ci,
        &mut db,
        Event::CiEvent(ci::Event::BuildFailed(
            CiId(1),
            memory_commit_b(),
            None,
        ))
    );
    assert_eq!(db.running.unwrap(), RunningEntry{
        pull_commit: memory_commit_c(),
        merge_commit: None,
        pr: memory_pr_b(),
        message: "M!".to_owned(),
        canceled: false,
        built: false,
    });
    assert!(db.queue.is_empty());
    assert!(vcs.borrow().master.is_none());
    assert_eq!(vcs.borrow().staging.as_ref().unwrap(), &memory_commit_c());
    assert_eq!(
        ui.borrow().results,
        vec![(memory_pr_a(), ui::Status::Failure(
            memory_commit_a(),
            memory_commit_b(),
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
        pull_commit: memory_commit_a(),
        merge_commit: Some(memory_commit_b()),
        pr: memory_pr_a(),
        canceled: false,
        message: "MSG!".to_owned(),
        built: false,
    }).unwrap();
    vcs.borrow_mut().staging = Some(memory_commit_b());
    handle_event(
        &mut ui,
        &mut vcs,
        &mut ci,
        &mut db,
        Event::CiEvent(ci::Event::BuildStarted(
            CiId(1),
            memory_commit_b(),
            Some("http://example.com/".into_url().expect("this to be valid")),
        ))
    );
    assert!(db.running.is_some());
    assert!(db.queue.is_empty());
    assert_eq!(
        ui.borrow().results,
        vec![(memory_pr_a(), ui::Status::Testing(
            memory_commit_a(),
            memory_commit_b(),
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
        pull_commit: memory_commit_a(),
        merge_commit: Some(memory_commit_b()),
        pr: memory_pr_a(),
        canceled: false,
        message: "MSG!".to_owned(),
        built: false,
    }).unwrap();
    vcs.borrow_mut().staging = Some(memory_commit_b());
    handle_event(
        &mut ui,
        &mut vcs,
        &mut ci,
        &mut db,
        Event::CiEvent(ci::Event::BuildSucceeded(
            CiId(1),
            memory_commit_b(),
            None,
        ))
    );
    assert!(db.running.is_some());
    assert!(db.queue.is_empty());
    assert_eq!(vcs.borrow().master.as_ref().unwrap(), &memory_commit_b());
    assert_eq!(
        ui.borrow().results,
        vec![
            (memory_pr_a(), ui::Status::Success(
                memory_commit_a(),
                memory_commit_b(),
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
        pull_commit: memory_commit_a(),
        merge_commit: Some(memory_commit_b()),
        pr: memory_pr_a(),
        canceled: false,
        message: "MSG!".to_owned(),
        built: false,
    }).unwrap();
    vcs.borrow_mut().staging = Some(memory_commit_b());
    handle_event(
        &mut ui,
        &mut vcs,
        &mut ci,
        &mut db,
        Event::CiEvent(ci::Event::BuildSucceeded(
            CiId(1),
            memory_commit_b(),
            None,
        ))
    );
    assert!(db.running.is_some());
    assert!(db.queue.is_empty());
    assert_eq!(vcs.borrow().master.as_ref().unwrap(), &memory_commit_b());
    assert_eq!(
        ui.borrow().results,
        vec![
            (memory_pr_a(), ui::Status::Success(
                memory_commit_a(),
                memory_commit_b(),
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
            CiId(1),
            memory_commit_b(),
            None,
        ))
    );
    assert!(db.running.is_some());
    assert!(db.queue.is_empty());
    assert_eq!(vcs.borrow().master.as_ref().unwrap(), &memory_commit_b());
    assert_eq!(
        ui.borrow().results,
        vec![
            (memory_pr_a(), ui::Status::Success(
                memory_commit_a(),
                memory_commit_b(),
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
        pull_commit: memory_commit_a(),
        merge_commit: Some(memory_commit_b()),
        pr: memory_pr_a(),
        canceled: false,
        message: "MSG!".to_owned(),
        built: true,
    }).unwrap();
    vcs.borrow_mut().staging = Some(memory_commit_b());
    handle_event(
        &mut ui,
        &mut vcs,
        &mut ci,
        &mut db,
        Event::VcsEvent(vcs::Event::FailedMoveToMaster(
            PipelineId(0),
            memory_commit_b(),
        ))
    );
    assert!(db.running.is_none());
    assert!(db.queue.is_empty());
    assert!(vcs.borrow().master.is_none());
    assert_eq!(vcs.borrow().staging.as_ref().unwrap(), &memory_commit_b());
    assert_eq!(
        ui.borrow().results,
        vec![(memory_pr_a(), ui::Status::Unmoveable(
            memory_commit_a(),
            memory_commit_b(),
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
        pull_commit: memory_commit_a(),
        merge_commit: Some(memory_commit_b()),
        pr: memory_pr_a(),
        canceled: false,
        message: "MSG!".to_owned(),
        built: true,
    }).unwrap();
    db.push_queue(PipelineId(0), QueueEntry{
        commit: memory_commit_c(),
        pr: memory_pr_b(),
        message: "M!".to_owned(),
    }).unwrap();
    vcs.borrow_mut().staging = Some(memory_commit_b());
    handle_event(
        &mut ui,
        &mut vcs,
        &mut ci,
        &mut db,
        Event::VcsEvent(vcs::Event::FailedMoveToMaster(
            PipelineId(0),
            memory_commit_b(),
        ))
    );
    assert_eq!(db.running.unwrap(), RunningEntry{
        pull_commit: memory_commit_c(),
        merge_commit: None,
        pr: memory_pr_b(),
        message: "M!".to_owned(),
        canceled: false,
        built: false,
    });
    assert!(db.queue.is_empty());
    assert!(vcs.borrow().master.is_none());
    assert_eq!(vcs.borrow().staging.as_ref().unwrap(), &memory_commit_c());
    assert_eq!(
        ui.borrow().results,
        vec![(memory_pr_a(), ui::Status::Unmoveable(
            memory_commit_a(),
            memory_commit_b(),
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
        pull_commit: memory_commit_a(),
        merge_commit: Some(memory_commit_b()),
        pr: memory_pr_a(),
        message: "MSG!".to_owned(),
        canceled: false,
        built: true,
    }).unwrap();
    vcs.borrow_mut().staging = Some(memory_commit_b());
    handle_event(
        &mut ui,
        &mut vcs,
        &mut ci,
        &mut db,
        Event::VcsEvent(vcs::Event::MovedToMaster(
            PipelineId(0),
            memory_commit_b(),
        ))
    );
    assert!(db.running.is_none());
    assert!(db.queue.is_empty());
    assert!(vcs.borrow().master.is_none());
    assert_eq!(
        ui.borrow().results,
        vec![(memory_pr_a(), ui::Status::Completed(
            memory_commit_a(),
            memory_commit_b(),
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
        pull_commit: memory_commit_a(),
        merge_commit: Some(memory_commit_b()),
        pr: memory_pr_a(),
        message: "MSG!".to_owned(),
        canceled: false,
        built: true,
    }).unwrap();
    db.push_queue(PipelineId(0), QueueEntry{
        commit: memory_commit_c(),
        pr: memory_pr_b(),
        message: "M!".to_owned(),
    }).unwrap();
    vcs.borrow_mut().staging = Some(memory_commit_b());
    handle_event(
        &mut ui,
        &mut vcs,
        &mut ci,
        &mut db,
        Event::VcsEvent(vcs::Event::MovedToMaster(
            PipelineId(0),
            memory_commit_b(),
        ))
    );
    assert_eq!(db.running.unwrap(), RunningEntry{
        pull_commit: memory_commit_c(),
        merge_commit: None,
        pr: memory_pr_b(),
        message: "M!".to_owned(),
        canceled: false,
        built: false,
    });
    assert!(db.queue.is_empty());
    assert!(vcs.borrow().master.is_none());
    assert_eq!(vcs.borrow().staging.as_ref().unwrap(), &memory_commit_c());
    assert_eq!(
        ui.borrow().results,
        vec![
            (memory_pr_a(), ui::Status::Completed(
                memory_commit_a(),
                memory_commit_b(),
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
        pull_commit: memory_commit_a(),
        merge_commit: Some(memory_commit_b()),
        pr: memory_pr_a(),
        message: "MSG!".to_owned(),
        canceled: false,
        built: false,
    }).unwrap();
    handle_event(
        &mut ui,
        &mut vcs,
        &mut ci,
        &mut db,
        Event::UiEvent(ui::Event::Canceled(
            PipelineId(0),
            memory_pr_a()
        ))
    );
    assert_eq!(db.running.unwrap(), RunningEntry{
        pull_commit: memory_commit_a(),
        merge_commit: Some(memory_commit_b()),
        pr: memory_pr_a(),
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
        pull_commit: memory_commit_a(),
        merge_commit: Some(memory_commit_b()),
        pr: memory_pr_a(),
        message: "MSG!".to_owned(),
        canceled: false,
        built: false,
    }).unwrap();
    handle_event(
        &mut ui,
        &mut vcs,
        &mut ci,
        &mut db,
        Event::UiEvent(ui::Event::Changed(
            PipelineId(0),
            memory_pr_a(),
            memory_commit_c(),
            "".to_owned(),
            Url::parse("http://www.com/").unwrap(),
        ))
    );
    assert_eq!(db.running.unwrap(), RunningEntry{
        pull_commit: memory_commit_a(),
        merge_commit: Some(memory_commit_b()),
        pr: memory_pr_a(),
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
        pull_commit: memory_commit_a(),
        merge_commit: Some(memory_commit_b()),
        pr: memory_pr_a(),
        message: "MSG!".to_owned(),
        canceled: false,
        built: false,
    }).unwrap();
    handle_event(
        &mut ui,
        &mut vcs,
        &mut ci,
        &mut db,
        Event::UiEvent(ui::Event::Changed(
            PipelineId(0),
            memory_pr_a(),
            memory_commit_a(),
            "".to_owned(),
            Url::parse("http://www.com/").unwrap(),
        ))
    );
    assert_eq!(db.running.unwrap(), RunningEntry{
        pull_commit: memory_commit_a(),
        merge_commit: Some(memory_commit_b()),
        pr: memory_pr_a(),
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
        pull_commit: memory_commit_a(),
        merge_commit: Some(memory_commit_b()),
        pr: memory_pr_a(),
        message: "MSG!".to_owned(),
        canceled: false,
        built: false,
    }).unwrap();
    db.push_queue(PipelineId(0), QueueEntry{
        commit: memory_commit_c(),
        pr: memory_pr_b(),
        message: "MSG!".to_owned(),
    }).unwrap();
    handle_event(
        &mut ui,
        &mut vcs,
        &mut ci,
        &mut db,
        Event::UiEvent(ui::Event::Changed(
            PipelineId(0),
            memory_pr_b(),
            memory_commit_d(),
            "".to_owned(),
            Url::parse("http://www.com/").unwrap(),
        ))
    );
    assert_eq!(db.running.unwrap(), RunningEntry{
        pull_commit: memory_commit_a(),
        merge_commit: Some(memory_commit_b()),
        pr: memory_pr_a(),
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
        pull_commit: memory_commit_a(),
        merge_commit: Some(memory_commit_b()),
        pr: memory_pr_a(),
        message: "MSG!".to_owned(),
        canceled: false,
        built: false,
    }).unwrap();
    db.push_queue(PipelineId(0), QueueEntry{
        commit: memory_commit_c(),
        pr: memory_pr_b(),
        message: "MSG!".to_owned(),
    }).unwrap();
    handle_event(
        &mut ui,
        &mut vcs,
        &mut ci,
        &mut db,
        Event::UiEvent(ui::Event::Changed(
            PipelineId(0),
            memory_pr_b(),
            memory_commit_c(),
            "".to_owned(),
            Url::parse("http://www.com/").unwrap(),
        ))
    );
    assert_eq!(db.running.unwrap(), RunningEntry{
        pull_commit: memory_commit_a(),
        merge_commit: Some(memory_commit_b()),
        pr: memory_pr_a(),
        canceled: false,
        built: false,
        message: "MSG!".to_owned(),
    });
    assert_eq!(db.queue[0], QueueEntry{
        commit: memory_commit_c(),
        pr: memory_pr_b(),
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
        pull_commit: memory_commit_a(),
        merge_commit: Some(memory_commit_b()),
        pr: memory_pr_a(),
        message: "MSG!".to_owned(),
        canceled: false,
        built: false,
    }).unwrap();
    handle_event(
        &mut ui,
        &mut vcs,
        &mut ci,
        &mut db,
        Event::UiEvent(ui::Event::Closed(
            PipelineId(0),
            memory_pr_a()
        ))
    );
    assert_eq!(db.running.unwrap(), RunningEntry{
        pull_commit: memory_commit_a(),
        merge_commit: Some(memory_commit_b()),
        pr: memory_pr_a(),
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
            memory_pr_a(),
            Some(memory_commit_a()),
            "Message!".to_owned(),
        ))
    );
    assert_eq!(vcs.borrow().staging, Some(memory_commit_a()));
    assert!(vcs.borrow().master.is_none());
    assert!(ci.borrow().build.is_none());
    assert_eq!(ui.borrow().results, vec![
        (memory_pr_a(), ui::Status::Approved(
            memory_commit_a(),
        )),
    ]);
    vcs.borrow_mut().staging = Some(memory_commit_b());
    handle_event(
        &mut ui,
        &mut vcs,
        &mut ci,
        &mut db,
        Event::VcsEvent(vcs::Event::MergedToStaging(
            PipelineId(0),
            memory_commit_a(),
            memory_commit_b(),
        ))
    );
    assert_eq!(db.running, Some(RunningEntry{
        pull_commit: memory_commit_a(),
        merge_commit: Some(memory_commit_b()),
        pr: memory_pr_a(),
        canceled: false,
        built: false,
        message: "Message!".to_owned(),
    }));
    assert!(db.queue.is_empty());
    assert_eq!(vcs.borrow().staging, Some(memory_commit_b()));
    assert!(vcs.borrow().master.is_none());
    assert_eq!(ci.borrow().build, Some(memory_commit_b()));
    assert_eq!(ui.borrow().results, vec![
        (memory_pr_a(), ui::Status::Approved(
            memory_commit_a(),
        )),
        (memory_pr_a(), ui::Status::StartingBuild(
            memory_commit_a(),
            memory_commit_b(),
        )),
    ]);
    handle_event(
        &mut ui,
        &mut vcs,
        &mut ci,
        &mut db,
        Event::CiEvent(ci::Event::BuildSucceeded(
            CiId(1),
            memory_commit_b(),
            None,
        ))
    );
    assert_eq!(vcs.borrow().staging, Some(memory_commit_b()));
    assert_eq!(vcs.borrow().master, Some(memory_commit_b()));
    assert_eq!(ui.borrow().results, vec![
        (memory_pr_a(), ui::Status::Approved(
            memory_commit_a(),
        )),
        (memory_pr_a(), ui::Status::StartingBuild(
            memory_commit_a(),
            memory_commit_b(),
        )),
        (memory_pr_a(), ui::Status::Success(
            memory_commit_a(),
            memory_commit_b(),
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
            memory_commit_b()
        ))
    );
    assert_eq!(vcs.borrow().staging, Some(memory_commit_b()));
    assert_eq!(vcs.borrow().master, Some(memory_commit_b()));
    assert_eq!(ui.borrow().results, vec![
        (memory_pr_a(), ui::Status::Approved(
            memory_commit_a(),
        )),
        (memory_pr_a(), ui::Status::StartingBuild(
            memory_commit_a(),
            memory_commit_b(),
        )),
        (memory_pr_a(), ui::Status::Success(
            memory_commit_a(),
            memory_commit_b(),
            None,
        )),
        (memory_pr_a(), ui::Status::Completed(
            memory_commit_a(),
            memory_commit_b(),
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
            memory_pr_a(),
            Some(memory_commit_a()),
            "MSG!".to_owned(),
        ))
    );
    assert_eq!(db.running, Some(RunningEntry{
        pull_commit: memory_commit_a(),
        merge_commit: None,
        pr: memory_pr_a(),
        canceled: false,
        built: false,
        message: "MSG!".to_owned(),
    }));
    assert_eq!(vcs.borrow().staging, Some(memory_commit_a()));
    assert!(vcs.borrow().master.is_none());
    assert!(ci.borrow().build.is_none());
    assert_eq!(ui.borrow().results, vec![
        (memory_pr_a(), ui::Status::Approved(
            memory_commit_a(),
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
            memory_pr_c(),
            Some(memory_commit_c()),
            "Message!".to_owned(),
        ))
    );
    assert_eq!(db.running, Some(RunningEntry{
        pull_commit: memory_commit_a(),
        merge_commit: None,
        pr: memory_pr_a(),
        canceled: false,
        built: false,
        message: "MSG!".to_owned(),
    }));
    assert_eq!(db.queue.len(), 1);
    // The first is now done merging. It should now be sent to the CI.
    vcs.borrow_mut().staging = Some(memory_commit_b());
    handle_event(
        &mut ui,
        &mut vcs,
        &mut ci,
        &mut db,
        Event::VcsEvent(vcs::Event::MergedToStaging(
            PipelineId(0),
            memory_commit_a(),
            memory_commit_b(),
        ))
    );
    assert_eq!(db.running, Some(RunningEntry{
        pull_commit: memory_commit_a(),
        merge_commit: Some(memory_commit_b()),
        pr: memory_pr_a(),
        canceled: false,
        built: false,
        message: "MSG!".to_owned(),
    }));
    assert_eq!(vcs.borrow().staging, Some(memory_commit_b()));
    assert!(vcs.borrow().master.is_none());
    assert_eq!(ci.borrow().build, Some(memory_commit_b()));
    assert_eq!(ui.borrow().results, vec![
        (memory_pr_a(), ui::Status::Approved(
            memory_commit_a(),
        )),
        (memory_pr_c(), ui::Status::Approved(
            memory_commit_c(),
        )),
        (memory_pr_a(), ui::Status::StartingBuild(
            memory_commit_a(),
            memory_commit_b(),
        )),
    ]);
    // The CI successfully built it. It should now be moved to master.
    handle_event(
        &mut ui,
        &mut vcs,
        &mut ci,
        &mut db,
        Event::CiEvent(ci::Event::BuildSucceeded(
            CiId(1),
            memory_commit_b(),
            None,
        ))
    );
    assert_eq!(vcs.borrow().staging, Some(memory_commit_b()));
    assert_eq!(vcs.borrow().master, Some(memory_commit_b()));
    assert_eq!(ui.borrow().results, vec![
        (memory_pr_a(), ui::Status::Approved(
            memory_commit_a(),
        )),
        (memory_pr_c(), ui::Status::Approved(
            memory_commit_c(),
        )),
        (memory_pr_a(), ui::Status::StartingBuild(
            memory_commit_a(),
            memory_commit_b(),
        )),
        (memory_pr_a(), ui::Status::Success(
            memory_commit_a(),
            memory_commit_b(),
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
            memory_commit_b()
        ))
    );
    assert_eq!(vcs.borrow().staging, Some(memory_commit_c()));
    assert_eq!(vcs.borrow().master, Some(memory_commit_b()));
    assert_eq!(ui.borrow().results, vec![
        (memory_pr_a(), ui::Status::Approved(
            memory_commit_a(),
        )),
        (memory_pr_c(), ui::Status::Approved(
            memory_commit_c(),
        )),
        (memory_pr_a(), ui::Status::StartingBuild(
            memory_commit_a(),
            memory_commit_b(),
        )),
        (memory_pr_a(), ui::Status::Success(
            memory_commit_a(),
            memory_commit_b(),
            None,
        )),
        (memory_pr_a(), ui::Status::Completed(
            memory_commit_a(),
            memory_commit_b(),
        )),
    ]);
    // The second one is now merged into staging; let's start building.
    vcs.borrow_mut().staging = Some(memory_commit_d());
    handle_event(
        &mut ui,
        &mut vcs,
        &mut ci,
        &mut db,
        Event::VcsEvent(vcs::Event::MergedToStaging(
            PipelineId(0),
            memory_commit_c(),
            memory_commit_d(),
        ))
    );
    assert_eq!(ci.borrow().build, Some(memory_commit_d()));
    assert_eq!(ui.borrow().results, vec![
        (memory_pr_a(), ui::Status::Approved(
            memory_commit_a(),
        )),
        (memory_pr_c(), ui::Status::Approved(
            memory_commit_c(),
        )),
        (memory_pr_a(), ui::Status::StartingBuild(
            memory_commit_a(),
            memory_commit_b(),
        )),
        (memory_pr_a(), ui::Status::Success(
            memory_commit_a(),
            memory_commit_b(),
            None,
        )),
        (memory_pr_a(), ui::Status::Completed(
            memory_commit_a(),
            memory_commit_b(),
        )),
        (memory_pr_c(), ui::Status::StartingBuild(
            memory_commit_c(),
            memory_commit_d(),
        )),
    ]);
}

#[test]
fn handle_runthrough_2_ci() {
    let mut ui = MemoryUi::new();
    let mut vcs = MemoryVcs::new();
    let mut ci1 = MemoryCi::new();
    let mut ci2 = MemoryCi::new();
    let mut db = MemoryDb::new();
    // Add a first item to the queue. This one should be built first.
    handle_event_2_ci(
        &mut ui,
        &mut vcs,
        &mut ci1,
        &mut ci2,
        &mut db,
        Event::UiEvent(ui::Event::Approved(
            PipelineId(0),
            memory_pr_a(),
            Some(memory_commit_a()),
            "MSG!".to_owned(),
        ))
    );
    assert_eq!(db.running, Some(RunningEntry{
        pull_commit: memory_commit_a(),
        merge_commit: None,
        pr: memory_pr_a(),
        canceled: false,
        built: false,
        message: "MSG!".to_owned(),
    }));
    assert_eq!(vcs.borrow().staging, Some(memory_commit_a()));
    assert!(vcs.borrow().master.is_none());
    assert!(ci1.borrow().build.is_none());
    assert_eq!(ui.borrow().results, vec![
        (memory_pr_a(), ui::Status::Approved(
            memory_commit_a(),
        )),
    ]);
    // Add a second item to the queue. Since the first is not done merging
    // into the staging area, the build state should not have changed.
    handle_event_2_ci(
        &mut ui,
        &mut vcs,
        &mut ci1,
        &mut ci2,
        &mut db,
        Event::UiEvent(ui::Event::Approved(
            PipelineId(0),
            memory_pr_c(),
            Some(memory_commit_c()),
            "Message!".to_owned(),
        ))
    );
    assert_eq!(db.running, Some(RunningEntry{
        pull_commit: memory_commit_a(),
        merge_commit: None,
        pr: memory_pr_a(),
        canceled: false,
        built: false,
        message: "MSG!".to_owned(),
    }));
    assert_eq!(db.queue.len(), 1);
    // The first is now done merging. It should now be sent to the CI.
    vcs.borrow_mut().staging = Some(memory_commit_b());
    handle_event_2_ci(
        &mut ui,
        &mut vcs,
        &mut ci1,
        &mut ci2,
        &mut db,
        Event::VcsEvent(vcs::Event::MergedToStaging(
            PipelineId(0),
            memory_commit_a(),
            memory_commit_b(),
        ))
    );
    assert_eq!(db.running, Some(RunningEntry{
        pull_commit: memory_commit_a(),
        merge_commit: Some(memory_commit_b()),
        pr: memory_pr_a(),
        canceled: false,
        built: false,
        message: "MSG!".to_owned(),
    }));
    assert_eq!(vcs.borrow().staging, Some(memory_commit_b()));
    assert!(vcs.borrow().master.is_none());
    assert_eq!(ci1.borrow().build, Some(memory_commit_b()));
    assert_eq!(ci2.borrow().build, Some(memory_commit_b()));
    assert_eq!(ui.borrow().results, vec![
        (memory_pr_a(), ui::Status::Approved(
            memory_commit_a(),
        )),
        (memory_pr_c(), ui::Status::Approved(
            memory_commit_c(),
        )),
        (memory_pr_a(), ui::Status::StartingBuild(
            memory_commit_a(),
            memory_commit_b(),
        )),
    ]);
    // The first CI successfully built it. It should still be running.
    handle_event_2_ci(
        &mut ui,
        &mut vcs,
        &mut ci1,
        &mut ci2,
        &mut db,
        Event::CiEvent(ci::Event::BuildSucceeded(
            CiId(1),
            memory_commit_b(),
            None,
        ))
    );
    assert_eq!(db.running, Some(RunningEntry{
        pull_commit: memory_commit_a(),
        merge_commit: Some(memory_commit_b()),
        pr: memory_pr_a(),
        canceled: false,
        built: false,
        message: "MSG!".to_owned(),
    }));
    assert_eq!(vcs.borrow().staging, Some(memory_commit_b()));
    assert!(vcs.borrow().master.is_none());
    assert_eq!(ui.borrow().results, vec![
        (memory_pr_a(), ui::Status::Approved(
            memory_commit_a(),
        )),
        (memory_pr_c(), ui::Status::Approved(
            memory_commit_c(),
        )),
        (memory_pr_a(), ui::Status::StartingBuild(
            memory_commit_a(),
            memory_commit_b(),
        )),
    ]);
    // The second CI successfully built it. It should now be moved to master.
    handle_event_2_ci(
        &mut ui,
        &mut vcs,
        &mut ci1,
        &mut ci2,
        &mut db,
        Event::CiEvent(ci::Event::BuildSucceeded(
            CiId(2),
            memory_commit_b(),
            None,
        ))
    );
    assert_eq!(vcs.borrow().staging, Some(memory_commit_b()));
    assert_eq!(vcs.borrow().master, Some(memory_commit_b()));
    assert_eq!(ui.borrow().results, vec![
        (memory_pr_a(), ui::Status::Approved(
            memory_commit_a(),
        )),
        (memory_pr_c(), ui::Status::Approved(
            memory_commit_c(),
        )),
        (memory_pr_a(), ui::Status::StartingBuild(
            memory_commit_a(),
            memory_commit_b(),
        )),
        (memory_pr_a(), ui::Status::Success(
            memory_commit_a(),
            memory_commit_b(),
            None,
        )),
    ]);
    // It has been successfully moved to master. The next build should
    // start, and this one should be reported complete.
    handle_event_2_ci(
        &mut ui,
        &mut vcs,
        &mut ci1,
        &mut ci2,
        &mut db,
        Event::VcsEvent(vcs::Event::MovedToMaster(
            PipelineId(0),
            memory_commit_b()
        ))
    );
    assert_eq!(vcs.borrow().staging, Some(memory_commit_c()));
    assert_eq!(vcs.borrow().master, Some(memory_commit_b()));
    assert_eq!(ui.borrow().results, vec![
        (memory_pr_a(), ui::Status::Approved(
            memory_commit_a(),
        )),
        (memory_pr_c(), ui::Status::Approved(
            memory_commit_c(),
        )),
        (memory_pr_a(), ui::Status::StartingBuild(
            memory_commit_a(),
            memory_commit_b(),
        )),
        (memory_pr_a(), ui::Status::Success(
            memory_commit_a(),
            memory_commit_b(),
            None,
        )),
        (memory_pr_a(), ui::Status::Completed(
            memory_commit_a(),
            memory_commit_b(),
        )),
    ]);
    // The second one is now merged into staging; let's start building.
    vcs.borrow_mut().staging = Some(memory_commit_d());
    handle_event_2_ci(
        &mut ui,
        &mut vcs,
        &mut ci1,
        &mut ci2,
        &mut db,
        Event::VcsEvent(vcs::Event::MergedToStaging(
            PipelineId(0),
            memory_commit_c(),
            memory_commit_d(),
        ))
    );
    assert_eq!(ci1.borrow().build, Some(memory_commit_d()));
    assert_eq!(ci2.borrow().build, Some(memory_commit_d()));
    assert_eq!(ui.borrow().results, vec![
        (memory_pr_a(), ui::Status::Approved(
            memory_commit_a(),
        )),
        (memory_pr_c(), ui::Status::Approved(
            memory_commit_c(),
        )),
        (memory_pr_a(), ui::Status::StartingBuild(
            memory_commit_a(),
            memory_commit_b(),
        )),
        (memory_pr_a(), ui::Status::Success(
            memory_commit_a(),
            memory_commit_b(),
            None,
        )),
        (memory_pr_a(), ui::Status::Completed(
            memory_commit_a(),
            memory_commit_b(),
        )),
        (memory_pr_c(), ui::Status::StartingBuild(
            memory_commit_c(),
            memory_commit_d(),
        )),
    ]);
}

#[test]
fn handle_runthrough_2_ci_first_failed() {
    let mut ui = MemoryUi::new();
    let mut vcs = MemoryVcs::new();
    let mut ci1 = MemoryCi::new();
    let mut ci2 = MemoryCi::new();
    let mut db = MemoryDb::new();
    // Add a first item to the queue. This one should be built first.
    handle_event_2_ci(
        &mut ui,
        &mut vcs,
        &mut ci1,
        &mut ci2,
        &mut db,
        Event::UiEvent(ui::Event::Approved(
            PipelineId(0),
            memory_pr_a(),
            Some(memory_commit_a()),
            "MSG!".to_owned(),
        ))
    );
    assert_eq!(db.running, Some(RunningEntry{
        pull_commit: memory_commit_a(),
        merge_commit: None,
        pr: memory_pr_a(),
        canceled: false,
        built: false,
        message: "MSG!".to_owned(),
    }));
    assert_eq!(vcs.borrow().staging, Some(memory_commit_a()));
    assert!(vcs.borrow().master.is_none());
    assert!(ci1.borrow().build.is_none());
    assert_eq!(ui.borrow().results, vec![
        (memory_pr_a(), ui::Status::Approved(
            memory_commit_a(),
        )),
    ]);
    // Add a second item to the queue. Since the first is not done merging
    // into the staging area, the build state should not have changed.
    handle_event_2_ci(
        &mut ui,
        &mut vcs,
        &mut ci1,
        &mut ci2,
        &mut db,
        Event::UiEvent(ui::Event::Approved(
            PipelineId(0),
            memory_pr_c(),
            Some(memory_commit_c()),
            "Message!".to_owned(),
        ))
    );
    assert_eq!(db.running, Some(RunningEntry{
        pull_commit: memory_commit_a(),
        merge_commit: None,
        pr: memory_pr_a(),
        canceled: false,
        built: false,
        message: "MSG!".to_owned(),
    }));
    assert_eq!(db.queue.len(), 1);
    // The first is now done merging. It should now be sent to the CI.
    vcs.borrow_mut().staging = Some(memory_commit_b());
    handle_event_2_ci(
        &mut ui,
        &mut vcs,
        &mut ci1,
        &mut ci2,
        &mut db,
        Event::VcsEvent(vcs::Event::MergedToStaging(
            PipelineId(0),
            memory_commit_a(),
            memory_commit_b(),
        ))
    );
    assert_eq!(db.running, Some(RunningEntry{
        pull_commit: memory_commit_a(),
        merge_commit: Some(memory_commit_b()),
        pr: memory_pr_a(),
        canceled: false,
        built: false,
        message: "MSG!".to_owned(),
    }));
    assert_eq!(vcs.borrow().staging, Some(memory_commit_b()));
    assert!(vcs.borrow().master.is_none());
    assert_eq!(ci1.borrow().build, Some(memory_commit_b()));
    assert_eq!(ci2.borrow().build, Some(memory_commit_b()));
    assert_eq!(ui.borrow().results, vec![
        (memory_pr_a(), ui::Status::Approved(
            memory_commit_a(),
        )),
        (memory_pr_c(), ui::Status::Approved(
            memory_commit_c(),
        )),
        (memory_pr_a(), ui::Status::StartingBuild(
            memory_commit_a(),
            memory_commit_b(),
        )),
    ]);
    // The first CI failed. It should start the second one, stopping the first.
    handle_event_2_ci(
        &mut ui,
        &mut vcs,
        &mut ci1,
        &mut ci2,
        &mut db,
        Event::CiEvent(ci::Event::BuildFailed(
            CiId(1),
            memory_commit_b(),
            None,
        ))
    );
    assert_eq!(db.running, Some(RunningEntry{
        pull_commit: memory_commit_c(),
        merge_commit: None,
        pr: memory_pr_c(),
        canceled: false,
        built: false,
        message: "Message!".to_owned(),
    }));
    assert_eq!(vcs.borrow().staging, Some(memory_commit_c()));
    assert!(vcs.borrow().master.is_none());
    assert_eq!(ui.borrow().results, vec![
        (memory_pr_a(), ui::Status::Approved(
            memory_commit_a(),
        )),
        (memory_pr_c(), ui::Status::Approved(
            memory_commit_c(),
        )),
        (memory_pr_a(), ui::Status::StartingBuild(
            memory_commit_a(),
            memory_commit_b(),
        )),
        (memory_pr_a(), ui::Status::Failure(
            memory_commit_a(),
            memory_commit_b(),
            None,
        )),
    ]);
    // The second CI successfully built it. It should not affect the state of anything.
    handle_event_2_ci(
        &mut ui,
        &mut vcs,
        &mut ci1,
        &mut ci2,
        &mut db,
        Event::CiEvent(ci::Event::BuildSucceeded(
            CiId(2),
            memory_commit_b(),
            None,
        ))
    );
    assert_eq!(vcs.borrow().staging, Some(memory_commit_c()));
    assert!(vcs.borrow().master.is_none());
    assert_eq!(ui.borrow().results, vec![
        (memory_pr_a(), ui::Status::Approved(
            memory_commit_a(),
        )),
        (memory_pr_c(), ui::Status::Approved(
            memory_commit_c(),
        )),
        (memory_pr_a(), ui::Status::StartingBuild(
            memory_commit_a(),
            memory_commit_b(),
        )),
        (memory_pr_a(), ui::Status::Failure(
            memory_commit_a(),
            memory_commit_b(),
            None,
        )),
    ]);
    // The second one is now merged into staging; let's start building.
    vcs.borrow_mut().staging = Some(memory_commit_d());
    handle_event_2_ci(
        &mut ui,
        &mut vcs,
        &mut ci1,
        &mut ci2,
        &mut db,
        Event::VcsEvent(vcs::Event::MergedToStaging(
            PipelineId(0),
            memory_commit_c(),
            memory_commit_d(),
        ))
    );
    assert_eq!(db.running, Some(RunningEntry{
        pull_commit: memory_commit_c(),
        merge_commit: Some(memory_commit_d()),
        pr: memory_pr_c(),
        canceled: false,
        built: false,
        message: "Message!".to_owned(),
    }));
    assert_eq!(ci1.borrow().build, Some(memory_commit_d()));
    assert_eq!(ci2.borrow().build, Some(memory_commit_d()));
    assert_eq!(ui.borrow().results, vec![
        (memory_pr_a(), ui::Status::Approved(
            memory_commit_a(),
        )),
        (memory_pr_c(), ui::Status::Approved(
            memory_commit_c(),
        )),
        (memory_pr_a(), ui::Status::StartingBuild(
            memory_commit_a(),
            memory_commit_b(),
        )),
        (memory_pr_a(), ui::Status::Failure(
            memory_commit_a(),
            memory_commit_b(),
            None,
        )),
        (memory_pr_c(), ui::Status::StartingBuild(
            memory_commit_c(),
            memory_commit_d(),
        )),
    ]);
}

#[test]
fn handle_runthrough_2_ci_second_failed() {
    let mut ui = MemoryUi::new();
    let mut vcs = MemoryVcs::new();
    let mut ci1 = MemoryCi::new();
    let mut ci2 = MemoryCi::new();
    let mut db = MemoryDb::new();
    // Add a first item to the queue. This one should be built first.
    handle_event_2_ci(
        &mut ui,
        &mut vcs,
        &mut ci1,
        &mut ci2,
        &mut db,
        Event::UiEvent(ui::Event::Approved(
            PipelineId(0),
            memory_pr_a(),
            Some(memory_commit_a()),
            "MSG!".to_owned(),
        ))
    );
    assert_eq!(db.running, Some(RunningEntry{
        pull_commit: memory_commit_a(),
        merge_commit: None,
        pr: memory_pr_a(),
        canceled: false,
        built: false,
        message: "MSG!".to_owned(),
    }));
    assert_eq!(vcs.borrow().staging, Some(memory_commit_a()));
    assert!(vcs.borrow().master.is_none());
    assert!(ci1.borrow().build.is_none());
    assert_eq!(ui.borrow().results, vec![
        (memory_pr_a(), ui::Status::Approved(
            memory_commit_a(),
        )),
    ]);
    // Add a second item to the queue. Since the first is not done merging
    // into the staging area, the build state should not have changed.
    handle_event_2_ci(
        &mut ui,
        &mut vcs,
        &mut ci1,
        &mut ci2,
        &mut db,
        Event::UiEvent(ui::Event::Approved(
            PipelineId(0),
            memory_pr_c(),
            Some(memory_commit_c()),
            "Message!".to_owned(),
        ))
    );
    assert_eq!(db.running, Some(RunningEntry{
        pull_commit: memory_commit_a(),
        merge_commit: None,
        pr: memory_pr_a(),
        canceled: false,
        built: false,
        message: "MSG!".to_owned(),
    }));
    assert_eq!(db.queue.len(), 1);
    // The first is now done merging. It should now be sent to the CI.
    vcs.borrow_mut().staging = Some(memory_commit_b());
    handle_event_2_ci(
        &mut ui,
        &mut vcs,
        &mut ci1,
        &mut ci2,
        &mut db,
        Event::VcsEvent(vcs::Event::MergedToStaging(
            PipelineId(0),
            memory_commit_a(),
            memory_commit_b(),
        ))
    );
    assert_eq!(db.running, Some(RunningEntry{
        pull_commit: memory_commit_a(),
        merge_commit: Some(memory_commit_b()),
        pr: memory_pr_a(),
        canceled: false,
        built: false,
        message: "MSG!".to_owned(),
    }));
    assert_eq!(vcs.borrow().staging, Some(memory_commit_b()));
    assert!(vcs.borrow().master.is_none());
    assert_eq!(ci1.borrow().build, Some(memory_commit_b()));
    assert_eq!(ci2.borrow().build, Some(memory_commit_b()));
    assert_eq!(ui.borrow().results, vec![
        (memory_pr_a(), ui::Status::Approved(
            memory_commit_a(),
        )),
        (memory_pr_c(), ui::Status::Approved(
            memory_commit_c(),
        )),
        (memory_pr_a(), ui::Status::StartingBuild(
            memory_commit_a(),
            memory_commit_b(),
        )),
    ]);
    // The first CI succeeded.
    handle_event_2_ci(
        &mut ui,
        &mut vcs,
        &mut ci1,
        &mut ci2,
        &mut db,
        Event::CiEvent(ci::Event::BuildSucceeded(
            CiId(1),
            memory_commit_b(),
            None,
        ))
    );
    assert_eq!(db.running, Some(RunningEntry{
        pull_commit: memory_commit_a(),
        merge_commit: Some(memory_commit_b()),
        pr: memory_pr_a(),
        canceled: false,
        built: false,
        message: "MSG!".to_owned(),
    }));
    assert_eq!(vcs.borrow().staging, Some(memory_commit_b()));
    assert!(vcs.borrow().master.is_none());
    assert_eq!(ui.borrow().results, vec![
        (memory_pr_a(), ui::Status::Approved(
            memory_commit_a(),
        )),
        (memory_pr_c(), ui::Status::Approved(
            memory_commit_c(),
        )),
        (memory_pr_a(), ui::Status::StartingBuild(
            memory_commit_a(),
            memory_commit_b(),
        )),
    ]);
    // The second CI failed. It should start on the second PR.
    handle_event_2_ci(
        &mut ui,
        &mut vcs,
        &mut ci1,
        &mut ci2,
        &mut db,
        Event::CiEvent(ci::Event::BuildFailed(
            CiId(2),
            memory_commit_b(),
            None,
        ))
    );
    assert_eq!(db.running, Some(RunningEntry{
        pull_commit: memory_commit_c(),
        merge_commit: None,
        pr: memory_pr_c(),
        canceled: false,
        built: false,
        message: "Message!".to_owned(),
    }));
    assert_eq!(vcs.borrow().staging, Some(memory_commit_c()));
    assert!(vcs.borrow().master.is_none());
    assert_eq!(ui.borrow().results, vec![
        (memory_pr_a(), ui::Status::Approved(
            memory_commit_a(),
        )),
        (memory_pr_c(), ui::Status::Approved(
            memory_commit_c(),
        )),
        (memory_pr_a(), ui::Status::StartingBuild(
            memory_commit_a(),
            memory_commit_b(),
        )),
        (memory_pr_a(), ui::Status::Failure(
            memory_commit_a(),
            memory_commit_b(),
            None,
        )),
    ]);
    // The second one is now merged into staging; let's start building.
    vcs.borrow_mut().staging = Some(memory_commit_d());
    handle_event_2_ci(
        &mut ui,
        &mut vcs,
        &mut ci1,
        &mut ci2,
        &mut db,
        Event::VcsEvent(vcs::Event::MergedToStaging(
            PipelineId(0),
            memory_commit_c(),
            memory_commit_d(),
        ))
    );
    assert_eq!(db.running, Some(RunningEntry{
        pull_commit: memory_commit_c(),
        merge_commit: Some(memory_commit_d()),
        pr: memory_pr_c(),
        canceled: false,
        built: false,
        message: "Message!".to_owned(),
    }));
    assert_eq!(ci1.borrow().build, Some(memory_commit_d()));
    assert_eq!(ci2.borrow().build, Some(memory_commit_d()));
    assert_eq!(ui.borrow().results, vec![
        (memory_pr_a(), ui::Status::Approved(
            memory_commit_a(),
        )),
        (memory_pr_c(), ui::Status::Approved(
            memory_commit_c(),
        )),
        (memory_pr_a(), ui::Status::StartingBuild(
            memory_commit_a(),
            memory_commit_b(),
        )),
        (memory_pr_a(), ui::Status::Failure(
            memory_commit_a(),
            memory_commit_b(),
            None,
        )),
        (memory_pr_c(), ui::Status::StartingBuild(
            memory_commit_c(),
            memory_commit_d(),
        )),
    ]);
}