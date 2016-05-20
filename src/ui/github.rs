// This file is released under the same terms as Rust itself.

use hyper;
use hyper::buffer::BufReader;
use hyper::client::{Client, IntoUrl, RequestBuilder};
use hyper::header::Headers;
use hyper::method::Method;
use hyper::net::{HttpListener, NetworkListener, NetworkStream};
use hyper::server::{Request, Response};
use hyper::status::StatusCode;
use pipeline::{self, PipelineId};
use serde_json;
use serde_json::{from_reader as json_from_reader, to_vec as json_to_vec};
use std;
use std::collections::HashMap;
use std::fmt::{self, Display, Formatter};
use std::io::BufWriter;
use std::num::ParseIntError;
use std::str::FromStr;
use std::sync::mpsc::{Sender, Receiver};
use std::thread;
use ui;
use vcs::git::Commit;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct Repo {
    pub owner: String,
    pub repo: String,
}

pub struct Worker {
    listen: String,
    host: String,
    repos: HashMap<Repo, PipelineId>,
    authorization: Vec<u8>,
    client: Client,
}

impl Worker {
    pub fn new(
        listen: String,
        host: String,
        token: String,
    ) -> Worker {
        let mut authorization: Vec<u8> = b"token ".to_vec();
        authorization.extend(token.bytes());
        Worker {
            listen: listen,
            host: host,
            repos: HashMap::new(),
            authorization: authorization,
            client: Client::default(),
        }
    }
    pub fn add_pipeline(&mut self, pipeline_id: PipelineId, repo: Repo) {
        self.repos.insert(repo, pipeline_id);
    }
}

impl Clone for Worker {
    fn clone(&self) -> Worker {
        Worker {
            listen: self.listen.clone(),
            host: self.host.clone(),
            repos: HashMap::new(),
            authorization: self.authorization.clone(),
            client: Client::default(),
        }
    }
}

impl pipeline::Worker<ui::Event<Commit, Pr>, ui::Message<Pr>> for Worker {
    fn run(
        &mut self,
        recv_msg: Receiver<ui::Message<Pr>>,
        mut send_event: Sender<ui::Event<Commit, Pr>>
    ) {
        let send_event_2 = send_event.clone();
        let self_2 = self.clone();
        thread::spawn(move || {
            self_2.run_webhook(send_event_2);
        });
        loop {
            self.handle_message(
                recv_msg.recv().expect("Pipeline went away"),
                &mut send_event,
            );
        }
    }
}

impl Worker {
    fn run_webhook(
        &self,
        send_event: Sender<ui::Event<Commit, Pr>>,
    ) {
        let mut listener = HttpListener::new(&self.listen[..]).expect("webhook");
        while let Ok(mut stream) = listener.accept() {
            let addr = stream.peer_addr()
                .expect("webhook client address");
            let mut stream_clone = stream.clone();
            let mut buf_read = BufReader::new(
                &mut stream_clone as &mut NetworkStream
            );
            let mut buf_write = BufWriter::new(&mut stream);
            let req = Request::new(&mut buf_read, addr)
                .expect("webhook Request");
            let mut head = Headers::new();
            let res = Response::new(&mut buf_write, &mut head);
            self.handle_webhook(req, res, &send_event);
        }
    }

    fn handle_webhook(
        &self,
        req: Request,
        mut res: Response,
        send_event: &Sender<ui::Event<Commit, Pr>>
    ) {
        let x_github_event = {
            if let Some(xges) = req.headers.get_raw("X-Github-Event") {
                if let Some(xge) = xges.get(0) {
                    xge.clone()
                } else {
                    vec![]
                }
            } else {
                vec![]
            }
        };
        #[derive(Serialize, Deserialize)]
        struct RepositoryDesc {
            name: String,
            owner: UserDesc,
        }
        #[derive(Serialize, Deserialize)]
        struct UserDesc {
            login: String,
        }
        match &x_github_event[..] {
            b"issue_comment" => {
                #[derive(Serialize, Deserialize)]
                struct IssueCommentPullRequest {
                    url: String,
                }
                #[derive(Serialize, Deserialize)]
                struct IssueCommentUser {
                    login: String,
                }
                #[derive(Serialize, Deserialize)]
                struct IssueCommentIssue {
                    number: u32,
                    title: String,
                    body: String,
                    pull_request: Option<IssueCommentPullRequest>,
                    state: String,
                    user: IssueCommentUser,
                }
                #[derive(Serialize, Deserialize)]
                struct IssueCommentComment {
                    user: IssueCommentUser,
                    body: String,
                }
                #[derive(Serialize, Deserialize)]
                struct CommentDesc {
                    issue: IssueCommentIssue,
                    comment: IssueCommentComment,
                    repository: RepositoryDesc,
                }
                if let Ok(desc) = json_from_reader::<_, CommentDesc>(req) {
                    *res.status_mut() = StatusCode::NoContent;
                    if let Err(e) = res.send(&[]) {
                        warn!("Failed to send response to Github comment: {:?}", e);
                    }
                    if let Some(_) = desc.issue.pull_request {
                        info!("Got pull request comment");
                        let repo = Repo{
                            owner: desc.repository.owner.login,
                            repo: desc.repository.name,
                        };
                        let pipeline_id = match self.repos.get(&repo) {
                            Some(pipeline_id) => pipeline_id,
                            None => {
                                warn!(
                                    "Got bad repo {:?}",
                                    repo
                                );
                                return;
                            }
                        };
                        if desc.issue.state == "closed" {
                            info!("Pull request is closed");
                        } else if desc.comment.body.contains("r+") {
                            info!("Pull request is APPROVED");
                            let pr = Pr(desc.issue.number);
                            match self.get_commit_for_pr(repo, pr) {
                                Ok(commit) => {
                                    info!("Got commit {}", commit);
                                    let message = format!(
                                        "#{} a=@{} r=@{}\n\n## {} ##\n\n{}",
                                        pr,
                                        desc.issue.user.login,
                                        desc.comment.user.login,
                                        desc.issue.title,
                                        desc.issue.body,
                                    );
                                    send_event.send(ui::Event::Approved(
                                        *pipeline_id,
                                        pr,
                                        commit,
                                        message
                                    )).expect("PR Approved: Pipeline error");
                                }
                                Err(e) => {
                                    warn!(
                                        "Failed to get commit for PR {}: {:?}",
                                        pr,
                                        e
                                    );
                                }
                            }
                        } else if desc.comment.body.contains("r-") {
                            info!("Pull request is CANCELED");
                            let pr = Pr(desc.issue.number);
                            send_event.send(ui::Event::Canceled(
                                *pipeline_id,
                                pr
                            )).expect("PR Canceled: Pipeline error");
                        } else {
                            info!("Pull request comment is not a command");
                        }
                    } else {
                        info!("Got issue comment; do nothing");
                    }
                } else {
                    warn!("Got invalid comment");
                    *res.status_mut() = StatusCode::BadRequest;
                    if let Err(e) = res.send(&[]) {
                        warn!("Failed to send response to Github bad comment: {:?}", e);
                    }
                }
            }
            b"pull_request" => {
                #[derive(Serialize, Deserialize)]
                struct PrDesc {
                    number: u32,
                }
                if let Ok(desc) = json_from_reader::<_, PrDesc>(req) {
                    info!("Got PR: {}", desc.number);
                    *res.status_mut() = StatusCode::NoContent;
                } else {
                    warn!("Got invalid PR");
                    *res.status_mut() = StatusCode::BadRequest;
                }
                if let Err(e) = res.send(&[]) {
                    warn!("Failed to send response to Github PR: {:?}", e);
                }
            }
            b"ping" => {
                #[derive(Serialize, Deserialize)]
                struct PingDesc {
                    zen: String,
                }
                if let Ok(desc) = json_from_reader::<_, PingDesc>(req) {
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
                    warn!("Failed to send response to Github unknown: {:?}", e);
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
        msg: ui::Message<Pr>,
        _: &mut Sender<ui::Event<Commit, Pr>>
    ) {
        match msg {
            ui::Message::SendResult(pipeline_id, pr, status) => {
                if let Err(e) = self.send_result_to_pr(pipeline_id, pr, status) {
                    warn!("Failed to send {:?} to pr {}: {:?}", status, pr, e)
                }
            }
        }
    }

    fn get_commit_for_pr(
        &self,
        repo: Repo,
        pr: Pr,
    ) -> Result<Commit, GithubRequestError> {
        let url = format!(
            "{}/repos/{}/{}/pulls/{}",
            self.host,
            repo.owner,
            repo.repo,
            pr
        );
        let resp = try!(self.authed_request(Method::Get, &url).send());
        if !resp.status.is_success() {
            return Err(GithubRequestError::HttpStatus(resp.status))
        }
        #[derive(Deserialize, Serialize)]
        struct PrBranch {
            sha: String,
        }
        #[derive(Deserialize, Serialize)]
        struct PrDesc {
            number: u32,
            head: PrBranch,
        }
        let desc: PrDesc = try!(json_from_reader(resp));
        assert_eq!(desc.number, pr.0);
        Commit::from_str(&desc.head.sha).map_err(|x| x.into())
    }

    fn send_result_to_pr(
        &self,
        pipeline_id: PipelineId,
        pr: Pr,
        status: ui::Status,
    ) -> Result<(), GithubRequestError> {
        let mut repo = None;
        for (r, p) in &self.repos {
            if *p == pipeline_id {
                repo = Some(r);
            }
        }
        let repo = match repo {
            Some(repo) => repo,
            None => {
                return Err(GithubRequestError::Pipeline(pipeline_id));
            }
        };
        let url = format!(
            "{}/repos/{}/{}/issues/{}/comments",
            self.host,
            repo.owner,
            repo.repo,
            pr
        );
        #[derive(Deserialize, Serialize)]
        struct Comment {
            body: String,
        }
        let comment = Comment {
            body: match status {
                ui::Status::InProgress => "Testing PR ...",
                ui::Status::Success => "Success",
                ui::Status::Failure => "Build failed",
                ui::Status::Unmergeable => "Merge conflict!",
                ui::Status::Unmoveable => "Internal error: fast-forward master",
            }.to_owned(),
        };
        let resp = try!(self.authed_request(Method::Post, &url)
            .body(&*try!(json_to_vec(&comment)))
            .send());
        if !resp.status.is_success() {
            return Err(GithubRequestError::HttpStatus(resp.status))
        }
        Ok(())
    }

    fn authed_request<U: IntoUrl>(
        &self,
        method: Method,
        url: U
    ) -> RequestBuilder {
        let mut headers = Headers::new();
        headers.set_raw("Accept", vec![b"application/vnd.github.v3+json".to_vec()]);
        headers.set_raw("Authorization", vec![self.authorization.clone()]);
        headers.set_raw("User-Agent", vec![b"aelita (hyper/0.9)".to_vec()]);
        self.client.request(method, url)
            .headers(headers)
    }
}

quick_error! {
    #[derive(Debug)]
    pub enum GithubRequestError {
        /// HTTP-level error
        HttpStatus(status: StatusCode) {}
        /// HTTP-level error
        Http(err: hyper::error::Error) {
            cause(err)
            from()
        }
        /// Integer parsing error
        Int(err: std::num::ParseIntError) {
            cause(err)
            from()
        }
        /// JSON error
        Json(err: serde_json::error::Error) {
            cause(err)
            from()
        }
        /// Repo not found for pipeline
        Pipeline(pipeline_id: PipelineId) {}
    }
}

#[derive(Copy, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Pr(u32);

impl ui::Pr for Pr {
    fn remote(&self) -> String {
        format!("pull/{}/head", self.0)
    }
}

impl Display for Pr {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        <u32 as Display>::fmt(&self.0, f)
    }
}

impl FromStr for Pr {
    type Err = ParseIntError;
    fn from_str(s: &str) -> Result<Pr, ParseIntError> {
        s.parse().map(|st| Pr(st))
    }
}

impl Into<String> for Pr {
    fn into(self) -> String {
        self.0.to_string()
    }
}
