// This file is released under the same terms as Rust itself.

use ci;
use crossbeam;
use hyper::Url;
use hyper::buffer::BufReader;
use hyper::header::Headers;
use hyper::net::{HttpListener, NetworkListener, NetworkStream};
use hyper::server::{Request, Response};
use hyper::status::StatusCode;
use pipeline::{self, PipelineId};
use serde_json::{from_slice as json_from_slice};
use std::collections::HashMap;
use std::io::BufWriter;
use std::str::FromStr;
use std::sync::mpsc::{Sender, Receiver};
use util::github_headers;
use vcs::git::Commit;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct Repo {
    pub owner: String,
    pub repo: String,
}

#[derive(Debug)]
struct RepoConfig {
    pipeline_id: PipelineId,
    context: String,
}

pub struct Worker {
    listen: String,
    pipelines: HashMap<Repo, Vec<RepoConfig>>,
    secret: String,
}

impl Worker {
    pub fn new(
        listen: String,
        secret: String,
    ) -> Worker {
        Worker {
            listen: listen,
            pipelines: HashMap::new(),
            secret: secret,
        }
    }
    pub fn add_pipeline(
        &mut self,
        pipeline_id: PipelineId,
        repo: Repo,
        context: String,
    ) {
        let repo_config = RepoConfig{
            pipeline_id: pipeline_id,
            context: context,
        };
        self.pipelines.entry(repo)
            .or_insert(Vec::new())
            .push(repo_config);
    }
}

// JSON API structs
#[derive(Serialize, Deserialize)]
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
#[derive(Serialize, Deserialize)]
struct RepositoryDesc {
    name: String,
    owner: OwnerDesc,
}
#[derive(Serialize, Deserialize)]
struct OwnerDesc {
    login: String,
}

impl pipeline::Worker<
    ci::Event<Commit>,
    ci::Message<Commit>,
> for Worker {
    fn run(
        &self,
        recv_msg: Receiver<ci::Message<Commit>>,
        mut send_event: Sender<ci::Event<Commit>>
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
        send_event: Sender<ci::Event<Commit>>,
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
        send_event: &Sender<ci::Event<Commit>>
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
                    };
                    if let Some(repo_configs) = self.pipelines.get(&repo) {
                        for repo_config in repo_configs {
                            let commit = match Commit::from_str(&desc.sha) {
                                Ok(commit) => commit,
                                Err(e) => {
                                    warn!(
                                        "Invalid commit {}: {:?}",
                                        desc.sha,
                                        e
                                    );
                                    return;
                                }
                            };
                            if repo_config.context == desc.context {
                                let event = match &desc.state[..] {
                                    "pending" => ci::Event::BuildStarted(
                                        repo_config.pipeline_id,
                                        commit,
                                        desc.target_url.as_ref().and_then(|u|
                                            Url::parse(&u[..]).ok()
                                        ),
                                    ),
                                    "failure" |
                                    "error" => ci::Event::BuildFailed(
                                        repo_config.pipeline_id,
                                        commit,
                                        desc.target_url.as_ref().and_then(|u|
                                            Url::parse(&u[..]).ok()
                                        ),
                                    ),
                                    "success" => ci::Event::BuildSucceeded(
                                        repo_config.pipeline_id,
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
                        }
                    } else {
                        warn!("Got status for unknown repo: {:?}", repo);
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
        msg: ci::Message<Commit>,
        _: &mut Sender<ci::Event<Commit>>,
    ) {
        match msg {
            // The build is triggered by Github itself on push.
            // There's nothing to do.
            ci::Message::StartBuild(_, _) => {}
        }
    }
}
