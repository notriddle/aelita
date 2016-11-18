// This file is released under the same terms as Rust itself.

mod auth;

use crossbeam;
use db::{self, Db, DbBox, PendingEntry, QueueEntry, RunningEntry, Transaction};
use horrorshow::prelude::*;
use hyper::buffer::BufReader;
use hyper::header::{ContentType, Headers};
use hyper::net::{HttpListener, HttpStream, NetworkListener, NetworkStream};
use hyper::server::{Request, Response};
use hyper::status::StatusCode;
use hyper::uri::RequestUri;
use pipeline::{self, PipelineId};
use quickersort::sort_by;
use spmc;
use std::borrow::Cow;
use std::convert::AsRef;
use std::error::Error;
use std::fmt::{self, Formatter};
use std::io::{BufWriter, Write};
use std::sync::mpsc::{Receiver, Sender};
use view::auth::AuthManager;

pub trait PipelinesConfig: Send + Sync + 'static {
    fn pipeline_by_name(&self, &str) -> Option<PipelineId>;
    fn all(&self) -> Vec<(Cow<str>, PipelineId)>;
}

pub use view::auth::{Auth, AuthRef};

const THREAD_COUNT: usize = 3;

pub struct Worker {
    listen: String,
    db_build: db::Builder,
    pipelines: Box<PipelinesConfig>,
    secret: String,
    auth: Auth,
}

impl Worker {
    pub fn new(
        listen: String,
        db_build: db::Builder,
        pipelines: Box<PipelinesConfig>,
        secret: String,
        auth: Auth,
    ) -> Self {
        Worker {
            listen: listen,
            db_build: db_build,
            pipelines: pipelines,
            secret: secret,
            auth: auth.into(),
        }
    }
}

#[derive(Clone)]
pub enum Event {}

#[derive(Clone)]
pub enum Message {}

impl pipeline::Worker<Event, Message> for Worker {
    fn run(&self, _recv: Receiver<Message>, _send: Sender<Event>) {
        let listen: &str = self.listen.as_ref();
        let secret: &str = self.secret.as_ref();
        let auth: AuthRef = (&self.auth).into();
        let pipelines = &*self.pipelines;
        let db_build = &self.db_build;
        crossbeam::scope(|scope| {
            let mut threads = Vec::with_capacity(THREAD_COUNT);
            for _ in 0..THREAD_COUNT {
                let (send, recv) = spmc::channel();
                scope.spawn(move || {
                    let mut thread = Thread {
                        db: db_build.open()
                            .expect("opening DB to succeed"),
                        pipelines: pipelines,
                        auth_manager: AuthManager{
                            auth: auth,
                            secret: secret,
                        },
                    };
                    thread.run(recv)
                });
                threads.push(send);
            }
            let mut listener = HttpListener::new(listen).expect("TCP socket");
            let mut i = 0;
            while let Ok(stream) = listener.accept() {
                threads[i].send(stream).unwrap();
                i += 1;
                i %= THREAD_COUNT;
            }
        });
    }
}

struct Thread<'a> {
    db: DbBox,
    pipelines: &'a PipelinesConfig,
    auth_manager: AuthManager<'a>,
}

impl<'a> Thread<'a> {
    fn run(&mut self, recv: spmc::Receiver<HttpStream>) {
        while let Ok(mut stream) = recv.recv() {
            let addr = stream.peer_addr()
                .expect("view client address");
            let mut stream_clone = stream.clone();
            let mut buf_read = BufReader::new(
                &mut stream_clone as &mut NetworkStream
            );
            let mut buf_write = BufWriter::new(&mut stream);
            let req = match Request::new(&mut buf_read, addr) {
                Ok(req) => req,
                Err(e) => {
                    warn!("Invalid view HTTP: {:?}", e);
                    continue;
                }
            };
            let mut head = Headers::new();
            let res = Response::new(&mut buf_write, &mut head);
            match self.handle_req(req, res) {
                Ok(()) => (),
                Err(e) => {
                    warn!("Failed to handle view request: {:?}", e);
                }
            }
        }
    }
    fn handle_req(
        &mut self,
        req: Request,
        res: Response,
    ) -> Result<(), Box<Error>> {
        let (req, mut res) = match self.auth_manager.check(req, res) {
            auth::CheckResult::Authenticated(req, res) => (req, res),
            auth::CheckResult::Err(e) => return Err(Box::new(e)),
            auth::CheckResult::NotAuthenticated => return Ok(()),
        };
        let pipeline = if let RequestUri::AbsolutePath(ref path) = req.uri {
            let mut path = &path[..];
            if path == "/" {
                None
            } else {
                if path.as_bytes()[0] == b'/' {
                    path = &path[1..];
                }
                match self.pipelines.pipeline_by_name(path) {
                    Some(pipeline_id) => {
                        *res.status_mut() = StatusCode::Ok;
                        Some((path.to_owned(), pipeline_id))
                    }
                    None if path == "style.css" => {
                        res.headers_mut().set(ContentType(mime!(Text/Css)));
                        let mut res = try!(res.start());
                        try!(res.write_all(include_bytes!("style.css")));
                        return Ok(());
                    }
                    None => {
                        *res.status_mut() = StatusCode::NotFound;
                        return Ok(());
                    }
                }
            }
        } else {
            *res.status_mut() = StatusCode::BadRequest;
            return Ok(());
        };
        res.headers_mut().set(ContentType::html());
        let mut res = try!(res.start());
        try!(res.write_all(br##"<!DOCTYPE html>"##));
        if let Some((name, pipeline_id)) = pipeline {
            self.handle_pipeline_req(&name, pipeline_id, req, res)
        } else {
            self.handle_home_req(req, res)
        }
    }
    fn handle_pipeline_req(
        &mut self,
        name: &str,
        pipeline_id: PipelineId,
        _req: Request,
        mut res: Response<::hyper::net::Streaming>,
    ) -> Result<(), Box<Error>> {
        let (pending_entries, queued_entries, running_entry) =
            try!(self.db.transaction(InfoTransaction{
                pipeline_id: pipeline_id
            }).wc());
        let is_empty = pending_entries.is_empty();
        let mut running = None;
        let mut queued = Vec::new();
        let pending: Vec<_> = pending_entries.into_iter().filter_map(|entry| {
            if Some(&entry.pr) == running_entry.as_ref().map(|x| &x.pr) {
                running = Some(entry);
            } else if queued_entries.iter()
                    .filter(|q| q.pr == entry.pr)
                    .next().is_some() {
                queued.push(entry);
            } else {
                return Some(entry);
            }
            None
        }).collect();
        let html = html!{
            html {
                head {
                    title { : name }
                    link(rel="stylesheet", href="https://cdnjs.cloudflare.com/ajax/libs/normalize/4.1.1/normalize.min.css");
                    link(rel="stylesheet", href="/style.css");
                    meta(name="viewport", content="width=device-width");
                }
                body {
                    h1 { : name }
                    table {
                        thead {
                            th { : "Status" }
                            th { : "PR#" }
                            th { : "Title" }
                        }
                        tbody {
                            |t| {
                                for entry in running {
                                    render_entry(State::Running, entry, t);
                                }
                                for entry in queued {
                                    render_entry(State::Queued, entry, t);
                                }
                                for entry in pending {
                                    render_entry(State::Pending, entry, t);
                                }
                                if is_empty {
                                    t << html!{
                                        td(colspan=3) {
                                            : "No opened pull requests"
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        };
        try!(html.write_to_io(&mut res));
        try!(res.end());
        Ok(())
    }
    fn handle_home_req(
        &mut self,
        _req: Request,
        mut res: Response<::hyper::net::Streaming>,
    ) -> Result<(), Box<Error>> {
        let mut pipelines = self.pipelines.all();
        sort_by(&mut pipelines, &|a, b| a.0.cmp(&b.0));
        let html = html!{
            html {
                head {
                    title { : "Aelita" }
                    link(rel="stylesheet", href="https://cdnjs.cloudflare.com/ajax/libs/normalize/4.1.1/normalize.min.css");
                    link(rel="stylesheet", href="/style.css");
                    meta(name="viewport", content="width=device-width");
                }
                body {
                    h1 { : "Repositories" }
                    table {
                        thead {
                            th { : "Name" }
                            th { : "Running" }
                            th { : "In queue" }
                            th { : "In review" }
                            th { : "Opened" }
                        }
                        tbody {
                            @ for &(ref n, pid) in &pipelines { |t| {
                                let n = &**n;
                                let (opened, queue, running) = 
                                    self.db.transaction(InfoTransaction{
                                        pipeline_id: pid
                                    }).unwrap_or((vec![], vec![], None));
                                let opened = opened.len();
                                let queue = queue.len();
                                let running = running.is_some();
                                let running = if running { 1 } else { 0 };
                                let review = opened - queue - running;
                                t << html!{
                                    tr {
                                        td(class="fill-link") {
                                            a(href=n) : { n }
                                        }
                                        td { : running }
                                        td { : queue }
                                        td { : review }
                                        td { : opened }
                                    }
                                }
                            }}
                            @ if pipelines.is_empty() {
                                td(colspan=5) {
                                    : "No configured repositories"
                                }
                            }
                        }
                    }
                    h2 { : "Github cheat sheet" }
                    p { : "To use the robot, say a command to it." }
                    dl {
                        dt { : Raw("<code>r+</code>") }
                        dd { : "Add the pull request to the merge queue." }
                        dt { : Raw("<code>r=@username</code>") }
                        dd { : "Add the pull request as \"username.\"" }
                        dt { : Raw("<code>r-</code>") }
                        dd { : "Cancel the pull request." }
                    }
                }
            }
        };
        try!(html.write_to_io(&mut res));
        try!(res.end());
        Ok(())
    }
}

struct InfoTransaction {
    pipeline_id: PipelineId,
}

impl Transaction for InfoTransaction {
    type Return = (
        Vec<PendingEntry>,
        Vec<QueueEntry>,
        Option<RunningEntry>,
    );
    fn run<D: Db>(
        self,
        db: &mut D
    ) -> Result<Self::Return, Box<Error + Send + Sync>> {
        retry!{{
            let pending_entries = retry_unwrap!(
                db.list_pending(self.pipeline_id)
            );
            let queued_entries = retry_unwrap!(
                db.list_queue(self.pipeline_id)
            );
            let running_entry = retry_unwrap!(
                db.peek_running(self.pipeline_id)
            );
            Ok((pending_entries, queued_entries, running_entry))
        }}
    }
}

/// Since there is no way to convert Box<Error+Send+Sync> to Box<Error>
/// without wrapping it, this is a hack to wrap it.
#[derive(Debug)]
struct WrapConvert (
    Box<Error + Send + Sync>,
);

impl Error for WrapConvert {
    fn description(&self) -> &str {
        self.0.description()
    }
    fn cause(&self) -> Option<&Error> {
        Some(&*self.0)
    }
}

impl fmt::Display for WrapConvert {
    fn fmt(&self, fmt: &mut Formatter) -> fmt::Result {
        self.0.fmt(fmt)
    }
}

trait WrapConvertable<T> {
    fn wc(self) -> Result<T, Box<Error>>;
}

impl<T> WrapConvertable<T> for Result<T, Box<Error + Send + Sync>> {
    fn wc(self) -> Result<T, Box<Error>> {
        self.map_err(|x| Box::new(WrapConvert(x)) as Box<Error>)
    }
}

enum State {
    Running,
    Queued,
    Pending,
}

fn render_entry(
    state: State,
    entry: PendingEntry,
    t: &mut TemplateBuffer,
) {
    t << html!{
        tr {
            td {
                : match state {
                    State::Running => "Running",
                    State::Queued => "In queue",
                    State::Pending => "In review",
                }
            }
            td(class="fill-link") {
                a(href=entry.url.to_string()) { : entry.pr.to_string() }
            }
            td { : &entry.title }
        }
    };
}
