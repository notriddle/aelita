// This file is released under the same terms as Rust itself.

use crossbeam;
use db::Db;
use db::sqlite::SqliteDb;
use hyper::buffer::BufReader;
use hyper::error::Error as HyperError;
use hyper::header::{ContentType, Headers};
use hyper::net::{HttpListener, NetworkListener, NetworkStream};
use hyper::server::{Request, Response};
use hyper::status::StatusCode;
use hyper::uri::RequestUri;
use pipeline::PipelineId;
use std::collections::{HashMap, HashSet};
use std::convert::AsRef;
use std::error::Error as StdError;
use std::io::{BufWriter, Write};
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use ui::Pr;
use vcs::Commit;

const WORKER_COUNT: usize = 1;

pub fn run_sqlite<C: Commit, P: Pr>(
    listen: String,
    path: PathBuf,
    pipelines: HashMap<String, PipelineId>,
)
    where <C as FromStr>::Err: StdError,
          <P as FromStr>::Err: StdError 
{
    let path: &Path = path.as_ref();
    let listen: &str = listen.as_ref();
    crossbeam::scope(|scope| {
        for _ in 0..WORKER_COUNT {
            let mut worker = Worker {
                db: SqliteDb::<C, P>::open(path).expect("opening sqlite to succeed"),
                pipelines: &pipelines,
                _commit: PhantomData::<C>,
                _pr: PhantomData::<P>,
            };
            scope.spawn(move || worker.run(listen));
        }
    });
}

struct Worker<'a, C, P, D>
    where C: Commit, P: Pr, D: Db<C, P>
{
    db: D,
    pipelines: &'a HashMap<String, PipelineId>,
    _commit: PhantomData<C>,
    _pr: PhantomData<P>,
}

impl<'a, C: Commit, P: Pr, D: Db<C, P>> Worker<'a, C, P, D> {
    fn run(&mut self, listen: &str) {
        let mut listener = HttpListener::new(listen).expect("a TCP socket");
        while let Ok(mut stream) = listener.accept() {
            let addr = stream.peer_addr()
                .expect("webhook client address");
            let mut stream_clone = stream.clone();
            let mut buf_read = BufReader::new(
                &mut stream_clone as &mut NetworkStream
            );
            let mut buf_write = BufWriter::new(&mut stream);
            let req = match Request::new(&mut buf_read, addr) {
                Ok(req) => req,
                Err(e) => {
                    warn!("Invalid webhook HTTP: {:?}", e);
                    continue;
                }
            };
            let mut head = Headers::new();
            let res = Response::new(&mut buf_write, &mut head);
            match self.handle_req(req, res) {
                Ok(()) => (),
                Err(e) => {
                    warn!("Failed to handle request: {:?}", e);
                }
            }
        }
    }
    fn handle_req(&mut self, req: Request, mut res: Response) -> Result<(), HyperError> {
        let pipeline_id = if let RequestUri::AbsolutePath(path) = req.uri {
            let mut path = &path[..];
            if path.as_bytes()[0] == b'/' {
                path = &path[1..];
            }
            match self.pipelines.get(path) {
                Some(pipeline_id) => {
                    *res.status_mut() = StatusCode::Ok;
                    *pipeline_id
                }
                None => {
                    *res.status_mut() = StatusCode::NotFound;
                    return Ok(());
                }
            }
        } else {
            *res.status_mut() = StatusCode::NotFound;
            return Ok(());
        };
        res.headers_mut().set(ContentType::html());
        let mut res = try!(res.start());
        let mut queued = HashSet::new();
        if let Some(running) = self.db.peek_running(pipeline_id) {
            let as_string = Into::<String>::into(running.pr);
            try!(res.write_all(br##"<h1>Running</h1>"##));
            try!(res.write_all(as_string.as_bytes()));
            queued.insert(as_string);
        }
        let queue = self.db.list_queue(pipeline_id);
        if !queue.is_empty() {
            try!(res.write_all(br##"<h1>Enqueued</h1><ul>"##));
            for entry in queue {
                let as_string = Into::<String>::into(entry.pr);
                try!(res.write_all(br##"<li>"##));
                try!(res.write_all(as_string.as_bytes()));
                queued.insert(as_string);
            }
            try!(res.write_all(br##"</ul>"##));
        }
        let pending = self.db.list_pending(pipeline_id);
        if !pending.is_empty() {
            try!(res.write_all(br##"<h1>Pending Review</h1><ul>"##));
            for entry in pending {
                let as_string = Into::<String>::into(entry.pr);
                if !queued.contains(&as_string) {
                    try!(res.write_all(br##"<li>"##));
                    try!(res.write_all(as_string.as_bytes()));
                }
            }
            try!(res.write_all(br##"</ul>"##));
        }
        try!(res.end());
        Ok(())
    }
}