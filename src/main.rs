// This file is released under the same terms as Rust itself.

//! An implementation of the Common Sense Rule of Software Engineering

#![feature(mpsc_select)]
#![feature(proc_macro)]
#![recursion_limit = "5000"]

#![feature(alloc_system)]
extern crate alloc_system;

extern crate crossbeam;
extern crate env_logger;
extern crate hex;
#[macro_use] extern crate horrorshow;
extern crate hyper;
#[macro_use] extern crate lazy_static;
#[macro_use] extern crate log;
#[macro_use] extern crate mime;
#[macro_use] extern crate openssl;
extern crate postgres;
#[macro_use] extern crate quick_error;
extern crate quickersort;
extern crate regex;
extern crate rest;
extern crate rusqlite;
extern crate serde;
extern crate serde_json;
#[macro_use] extern crate serde_derive;
extern crate spmc;
extern crate toml;
extern crate url;
extern crate void;

#[macro_use] mod util;
mod ci;
mod config;
mod db;
mod pipeline;
mod ui;
mod view;
mod vcs;

use config::WorkerBuilder;
use db::Db;
use pipeline::{Ci, Event, GetPipelineId, Pipeline, Ui, Vcs};
use std::borrow::Cow;
use std::env::args;
use std::error::Error;

fn main() {
    env_logger::init().unwrap();
    let mut args = args();
    let _ = args.next(); // ignore executable name
    let config_path = args.next()
        .map(Cow::Owned)
        .unwrap_or(Cow::Borrowed("config.toml"));
    let config_path = &*config_path;
    if config_path == "-12" {
        let worker_builder =
            config::twelvef::GithubBuilder::build_from_os_env()
            .unwrap();
        run_workers(worker_builder)
    } else {
        let worker_builder = config::toml::GithubBuilder::build_from_file(
            config_path
        ).unwrap();
        run_workers(worker_builder)
    }
}

fn run_workers<B: WorkerBuilder>(builder: B) -> ! {
    use std::sync::mpsc::{Select, Handle};
    let (workers, mut db) = builder.start();
    debug!(
        "Created {} pipelines, {} CIs, {} UIs, and {} VCSs (View: {})",
        workers.pipelines.len(),
        workers.cis.len(),
        workers.uis.len(),
        workers.vcss.len(),
        workers.view.is_some(),
    );
    unsafe {
        let select = Select::new();
        let mut ci_handles: Vec<Handle<ci::Event>> =
            workers.cis.iter().map(|worker| {
                select.handle(&worker.recv_event)
            }).collect();
        let mut ui_handles: Vec<Handle<ui::Event>> =
            workers.uis.iter().map(|worker| {
                select.handle(&worker.recv_event)
            }).collect();
        let mut vcs_handles: Vec<Handle<vcs::Event>> =
            workers.vcss.iter().map(|worker| {
                select.handle(&worker.recv_event)
            }).collect();
        let mut view_handle: Option<Handle<view::Event>> =
            workers.view.as_ref().map(|worker| {
                select.handle(&worker.recv_event)
            });
        // We cannot call add while collecting because the handle is moved.
        for h in &mut ci_handles { h.add(); }
        for h in &mut ui_handles { h.add(); }
        for h in &mut vcs_handles { h.add(); }
        if let Some(ref mut h) = view_handle { h.add(); }
        let mut pending: Option<Event> = None;
        'outer: loop {
            if let Some(event) = pending.take() {
                let pipeline_id = event.pipeline_id(&*workers.pipelines);
                let pipeline = workers.pipeline_by_id(pipeline_id);
                if let Some(pipeline) = pipeline {
                    let result = db.transaction(PipelineTransaction{
                        pipeline: pipeline,
                        event: event,
                    });
                    if let Err(e) = result {
                        warn!("Event handling failed: {:?}", e);
                    }
                }
            }
            let id = select.wait();
            for h in &mut ci_handles {
                if h.id() == id {
                    pending = h.recv().map(Event::CiEvent).ok();
                    continue 'outer;
                }
            }
            for h in &mut ui_handles { 
                if h.id() == id {
                    pending = h.recv().map(Event::UiEvent).ok();
                    continue 'outer;
                }
            }
            for h in &mut vcs_handles { 
                if h.id() == id {
                    pending = h.recv().map(Event::VcsEvent).ok();
                    continue 'outer;
                }
            }
        }
    }
}

struct PipelineTransaction<'cntx, C, U, V>
    where C: Ci + 'cntx,
          U: Ui + 'cntx,
          V: Vcs + 'cntx {
    pipeline: Pipeline<'cntx, C, U, V>,
    event: Event,
}

impl<'cntx, C, U, V> db::Transaction
        for PipelineTransaction<'cntx, C, U, V>
    where C: Ci + 'cntx,
          U: Ui + 'cntx,
          V: Vcs + 'cntx {
    type Return = ();
    fn run<D: Db>(
        mut self,
        db: &mut D
    ) -> Result<(), Box<Error + Send + Sync>> {
        use std::thread;
        use std::time::Duration;
        use util::{MIN_DELAY_SEC, MAX_DELAY_SEC};
        let mut delay = Duration::new(MIN_DELAY_SEC, 0);
        let max = Duration::new(MAX_DELAY_SEC, 0);
        let mut result;
        loop {
            result = self.pipeline.handle_event(db, self.event.clone());
            if let Err(ref e) = result {
                if delay <= max {
                    warn!("Retry handling event in {:?}: {:?}", delay, e);
                    thread::sleep(delay);
                    delay = delay * 2;
                    continue;
                }
            }
            break;
        }
        result
    }
}