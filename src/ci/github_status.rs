// This file is released under the same terms as Rust itself.

use ci::{self, CiId};
use crossbeam;
use hyper::Url;
use hyper::buffer::BufReader;
use hyper::header::Headers;
use hyper::net::{HttpListener, NetworkListener, NetworkStream};
use hyper::server::{Request, Response};
use hyper::status::StatusCode;
use pipeline;
use serde_json::{from_slice as json_from_slice};
use std::io::BufWriter;
use std::sync::mpsc::{Sender, Receiver};
use util::github_headers;

pub trait PipelinesConfig: Send + Sync + 'static {
    fn repo_by_id(&self, CiId) -> Option<Repo>;
    fn ids_by_repo(&self, &Repo) -> Vec<CiId>;
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct Repo {
    pub owner: String,
    pub repo: String,
    pub context: String,
}

pub struct Worker {
    listen: String,
    pipelines: Box<PipelinesConfig>,
    secret: String,
}

impl Worker {
    pub fn new(
        listen: String,
        secret: String,
        pipelines: Box<PipelinesConfig>,
    ) -> Worker {
        Worker {
            listen: listen,
            pipelines: pipelines,
            secret: secret,
        }
    }}


// JSON API structs
#[derive(Deserialize, Serialize)]
struct PingDesc {
    zen: String,
}
#[derive(Deserialize, Serialize)]
struct StatusDesc {
    state: String,
    target_url: Option<String>,
    context: String,
    sha: String,
    repository: RepositoryDesc,
}
#[derive(Deserialize, Serialize)]
struct RepositoryDesc {
    name: String,
    owner: OwnerDesc,
}
#[derive(Deserialize, Serialize)]
struct OwnerDesc {
    login: String,
}

impl pipeline::Worker<
    ci::Event,
    ci::Message,
> for Worker {
    fn run(
        &self,
        recv_msg: Receiver<ci::Message>,
        mut send_event: Sender<ci::Event>
    ) {
        crossbeam::scope(|scope| {
            let s2 = &*self;
            let send_event_2 = send_event.clone();
            scope.spawn(move || {
                s2.run_webhook(send_event_2);
            });
            loop {
                s2.handle_message(
                    recv_msg.recv().expect("Pipeline went away"),
                    &mut send_event,
                );
            }
        })
    }
}

impl Worker {
    fn run_webhook(
        &self,
        send_event: Sender<ci::Event>,
    ) {
        let mut listener = HttpListener::new(&self.listen[..])
            .expect("webhook");
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
            self.handle_webhook(req, res, &send_event);
        }
    }

    fn handle_webhook(
        &self,
        mut req: Request,
        mut res: Response,
        send_event: &Sender<ci::Event>
    ) {
        let head = github_headers::parse(&mut req, self.secret.as_bytes());
        let (x_github_event, body) = match head {
            Some(head) => head,
            None => return,
        };
        match &x_github_event[..] {
            b"status" => {
                if let Ok(desc) = json_from_slice::<StatusDesc>(&body) {
                    *res.status_mut() = StatusCode::NoContent;
                    if let Err(e) = res.send(&[]) {
                        warn!(
                            "Failed to send response to Github status: {:?}",
                            e,
                        );
                    }
                    let repo = Repo{
                        repo: desc.repository.name,
                        owner: desc.repository.owner.login,
                        context: desc.context,
                    };
                    let ids = self.pipelines.ids_by_repo(&repo);
                    if ids.is_empty() {
                        warn!("Got status for unknown repo: {:?}", repo);
                    }
                    for id in ids {
                        let commit = desc.sha.clone().into();
                        let event = match &desc.state[..] {
                            "pending" => ci::Event::BuildStarted(
                                id,
                                commit,
                                desc.target_url.as_ref().and_then(|u|
                                    Url::parse(&u[..]).ok()
                                ),
                            ),
                            "failure" |
                            "error" => ci::Event::BuildFailed(
                                id,
                                commit,
                                desc.target_url.as_ref().and_then(|u|
                                    Url::parse(&u[..]).ok()
                                ),
                            ),
                            "success" => ci::Event::BuildSucceeded(
                                id,
                                commit,
                                desc.target_url.as_ref().and_then(|u|
                                    Url::parse(&u[..]).ok()
                                ),
                            ),
                            _ => {
                                warn!(
                                    "Unknown status state: {}",
                                    desc.state
                                );
                                return;
                            },
                        };
                        send_event.send(event).expect("pipeline");
                    }
                } else {
                    warn!("Got invalid status");
                    *res.status_mut() = StatusCode::BadRequest;
                    if let Err(e) = res.send(&[]) {
                        warn!(
                            "Failed to send response to bad status: {:?}",
                            e,
                        );
                    }
                }
            }
            b"ping" => {
                if let Ok(desc) = json_from_slice::<PingDesc>(&body) {
                    info!("Got Ping: {}", desc.zen);
                    *res.status_mut() = StatusCode::NoContent;
                } else {
                    warn!("Got invalid Ping");
                    *res.status_mut() = StatusCode::BadRequest;
                }
                if let Err(e) = res.send(&[]) {
                    warn!("Failed to send response to Github ping: {:?}", e);
                }
            }
            e => {
                *res.status_mut() = StatusCode::BadRequest;
                if let Err(e) = res.send(&[]) {
                    warn!(
                        "Failed to send response to Github unknown: {:?}",
                        e,
                    );
                }
                warn!(
                    "Got Unknown Event {}",
                    String::from_utf8_lossy(&e)
                );
            }
        }
    }

    fn handle_message(
        &self,
        msg: ci::Message,
        _: &mut Sender<ci::Event>,
    ) {
        match msg {
            // The build is triggered by Github itself on push.
            // There's nothing to do.
            ci::Message::StartBuild(_, _) => {}
        }
    }
}
