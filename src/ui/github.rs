// This file is released under the same terms as Rust itself.

use crossbeam;
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
use std::collections::HashSet;
use std::fmt::{self, Display, Formatter};
use std::io::BufWriter;
use std::num::ParseIntError;
use std::str::FromStr;
use std::sync::mpsc::{Sender, Receiver};
use std::sync::Mutex;
use std::thread;
use std::time;
use ui::{self, comments};
use vcs::git::Commit;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct Repo {
    pub owner: String,
    pub repo: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RepoConfig {
    pipeline_id: PipelineId,
    teams_with_write: Option<HashSet<u32>>,
}

pub struct Worker {
    listen: String,
    host: String,
    repos: HashMap<Repo, RepoConfig>,
    authorization: Vec<u8>,
    client: Client,
    user_ident: String,
    rate_limit_until: Mutex<Option<time::SystemTime>>,
}

impl Worker {
    pub fn new(
        listen: String,
        host: String,
        token: String,
        user: String,
    ) -> Worker {
        let mut authorization: Vec<u8> = b"token ".to_vec();
        authorization.extend(token.bytes());
        let user_ident = format!("@{}", user);
        Worker {
            listen: listen,
            host: host,
            repos: HashMap::new(),
            authorization: authorization,
            user_ident: user_ident,
            client: Client::default(),
            rate_limit_until: Mutex::new(None),
        }
    }
    pub fn add_pipeline(&mut self, pipeline_id: PipelineId, repo: Repo) {
        let teams_with_write = if self.repo_is_org(&repo).expect("if org") {
            Some(self.get_all_teams_with_write(&repo).expect("Get team info"))
        } else {
            None
        };
        self.repos.insert(repo, RepoConfig{
            pipeline_id: pipeline_id,
            teams_with_write: teams_with_write,
        });
    }
}

// JSON API structs
#[derive(Serialize, Deserialize)]
struct IssueCommentPullRequest {
    url: String,
}
#[derive(Serialize, Deserialize)]
struct IssueCommentIssue {
    number: u32,
    title: String,
    body: String,
    pull_request: Option<IssueCommentPullRequest>,
    state: String,
    user: UserDesc,
}
#[derive(Serialize, Deserialize)]
struct IssueCommentComment {
    user: UserDesc,
    body: String,
}
#[derive(Serialize, Deserialize)]
struct PostCommentComment {
    body: String,
}
#[derive(Serialize, Deserialize)]
struct CommentDesc {
    issue: IssueCommentIssue,
    comment: IssueCommentComment,
    repository: RepositoryDesc,
}
#[derive(Serialize, Deserialize)]
struct RepositoryDesc {
    name: String,
    owner: OwnerDesc,
}
#[derive(Serialize, Deserialize)]
struct UserDesc {
    login: String,
    // type is a reserved word.
    #[serde(rename="type")]
    user_type: String,
}
#[derive(Serialize, Deserialize)]
struct OwnerDesc {
    login: String,
    // type is a reserved word.
    #[serde(rename="type")]
    owner_type: String,
}
#[derive(Serialize, Deserialize)]
struct PingDesc {
    zen: String,
}
#[derive(Serialize, Deserialize)]
struct TeamDesc {
    slug: String,
    id: u32,
}
#[derive(Serialize, Deserialize)]
struct TeamRepoDesc {
    permissions: TeamRepoPermissions,
}
#[derive(Serialize, Deserialize)]
struct TeamRepoPermissions {
    admin: bool,
    push: bool,
    pull: bool,
}
#[derive(Deserialize, Serialize)]
struct PrBranchDesc {
    sha: String,
}
#[derive(Deserialize, Serialize)]
struct PrDesc {
    number: u32,
    head: PrBranchDesc,
}

enum AcceptType {
    Regular,
    Repository,
}

impl pipeline::Worker<ui::Event<Commit, Pr>, ui::Message<Pr>> for Worker {
    fn run(
        &mut self,
        recv_msg: Receiver<ui::Message<Pr>>,
        mut send_event: Sender<ui::Event<Commit, Pr>>
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
        send_event: Sender<ui::Event<Commit, Pr>>,
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
        match &x_github_event[..] {
            b"issue_comment" => {
                if let Ok(desc) = json_from_reader::<_, CommentDesc>(req) {
                    *res.status_mut() = StatusCode::NoContent;
                    if let Err(e) = res.send(&[]) {
                        warn!(
                            "Failed to send response to Github comment: {:?}",
                            e,
                        );
                    }
                    if !desc.comment.body.contains(&self.user_ident) {
                        info!("Comment does not mention me; do nothing");
                    } else if desc.issue.state == "closed" {
                        info!("Comment is for closed issue; do nothing");
                    } else if let Some(_) = desc.issue.pull_request {
                        info!("Got pull request comment");
                        self.handle_pr_comment(send_event, desc);
                    } else {
                        info!("Got issue comment; do nothing");
                    }
                } else {
                    warn!("Got invalid comment");
                    *res.status_mut() = StatusCode::BadRequest;
                    if let Err(e) = res.send(&[]) {
                        warn!(
                            "Failed to send response to bad comment: {:?}",
                            e,
                        );
                    }
                }
            }
            b"ping" => {
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

    fn handle_pr_comment(
        &self,
        send_event: &Sender<ui::Event<Commit, Pr>>,
        desc: CommentDesc,
    ) {
        let repo = Repo{
            owner: desc.repository.owner.login,
            repo: desc.repository.name,
        };
        let pr = Pr(desc.issue.number);
        let repo_config = match self.repos.get(&repo) {
            Some(repo_config) => repo_config,
            None => {
                warn!(
                    "Got bad repo {:?}",
                    repo
                );
                return;
            }
        };
        let user = &desc.comment.user.login;
        let body = &desc.comment.body;
        let allowed = self.user_has_write(user, &repo, repo_config)
            .unwrap_or_else(|e| {
                warn!("Failed to check if {} has permission: {:?}", user, e);
                false
            });
        if !allowed {
            info!("Got mentioned by not-permitted user");
        } else if let Some(command) = comments::parse(&body, user) {
            self.handle_comment_command(
                send_event,
                command,
                &desc.issue,
                &repo,
                repo_config,
                &pr,
            );
        } else {
            info!("Pull request comment is not a command");
        }
    }

    fn handle_comment_command(
        &self,
        send_event: &Sender<ui::Event<Commit, Pr>>,
        command: comments::Command,
        issue: &IssueCommentIssue,
        repo: &Repo,
        repo_config: &RepoConfig,
        pr: &Pr,
    ) {
        match command {
            comments::Command::Approved(user) => {
                self.handle_approved_pr(
                    send_event,
                    issue,
                    repo,
                    repo_config,
                    pr,
                    user,
                );
            }
            comments::Command::Canceled => {
                self.handle_canceled_pr(send_event, repo_config, pr);
            }
        }
    }

    fn handle_approved_pr(
        &self,
        send_event: &Sender<ui::Event<Commit, Pr>>,
        issue: &IssueCommentIssue,
        repo: &Repo,
        repo_config: &RepoConfig,
        pr: &Pr,
        user: &str
    ) {
        match self.get_commit_for_pr(repo, pr) {
            Ok(commit) => {
                info!("Got commit {}", commit);
                let message = format!(
                    "#{} a=@{} r=@{}\n\n## {} ##\n\n{}",
                    pr,
                    issue.user.login,
                    user,
                    issue.title,
                    issue.body,
                );
                send_event.send(ui::Event::Approved(
                    repo_config.pipeline_id,
                    *pr,
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
    }

    fn handle_canceled_pr(
        &self,
        send_event: &Sender<ui::Event<Commit, Pr>>,
        repo_config: &RepoConfig,
        pr: &Pr,
    ) {
        send_event.send(ui::Event::Canceled(
            repo_config.pipeline_id,
            *pr,
        )).expect("PR Canceled: Pipeline error");
    }

    fn handle_message(
        &self,
        msg: ui::Message<Pr>,
        _: &mut Sender<ui::Event<Commit, Pr>>
    ) {
        match msg {
            ui::Message::SendResult(pipeline_id, pr, status) => {
                let result = self.send_result_to_pr(pipeline_id, pr, status);
                if let Err(e) = result {
                    warn!("Failed to send {:?} to pr {}: {:?}", status, pr, e)
                }
            }
        }
    }

    fn user_has_write(
        &self,
        user: &str,
        repo: &Repo,
        repo_config: &RepoConfig,
    ) -> Result<bool, GithubRequestError> {
        if let Some(ref teams_with_write) = repo_config.teams_with_write {
            info!("Using teams permission check");
            let mut allowed = false;
            for team in teams_with_write {
                if try!(self.user_is_member_of(user, *team)) {
                    allowed = true;
                    break;
                }
            }
            Ok(allowed)
        } else {
            info!("Using users permission check");
            self.user_is_collaborator_for(user, repo)
        }
    }

    fn get_commit_for_pr(
        &self,
        repo: &Repo,
        pr: &Pr,
    ) -> Result<Commit, GithubRequestError> {
        let url = format!(
            "{}/repos/{}/{}/pulls/{}",
            self.host,
            repo.owner,
            repo.repo,
            pr
        );
        let resp = try!(self.retry_send(|| {
            self.authed_request(
                Method::Get,
                AcceptType::Regular,
                &url
            )
        }));
        if !resp.status.is_success() {
            return Err(GithubRequestError::HttpStatus(resp.status))
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
        for (r, c) in &self.repos {
            if c.pipeline_id == pipeline_id {
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
        let comment = PostCommentComment {
            body: match status {
                ui::Status::InProgress => "Testing PR ...",
                ui::Status::Success => "Success",
                ui::Status::Failure => "Build failed",
                ui::Status::Unmergeable => "Merge conflict!",
                ui::Status::Unmoveable =>
                    "Internal error while fast-forward master",
            }.to_owned(),
        };
        let comment_body = try!(json_to_vec(&comment));
        let resp = try!(self.retry_send(|| {
            self.authed_request(Method::Post, AcceptType::Regular, &url)
                .body(&*comment_body)
        }));
        if !resp.status.is_success() {
            return Err(GithubRequestError::HttpStatus(resp.status))
        }
        Ok(())
    }

    fn user_is_member_of(
        &self,
        user: &str,
        team: u32,
    ) -> Result<bool, GithubRequestError> {
        let url = format!(
            "{}/teams/{}/members/{}",
            self.host,
            team,
            user,
        );
        let resp = try!(self.retry_send(|| {
            self.authed_request(
                Method::Get,
                AcceptType::Regular,
                &url
            )
        }));
        if resp.status == StatusCode::NotFound {
            Ok(false)
        } else if resp.status.is_success() {
            Ok(true)
        } else {
            Err(GithubRequestError::HttpStatus(resp.status))
        }
    }

    fn user_is_collaborator_for(
        &self,
        user: &str,
        repo: &Repo,
    ) -> Result<bool, GithubRequestError> {
        let url = format!(
            "{}/repos/{}/{}/collaborators/{}",
            self.host,
            repo.owner,
            repo.repo,
            user,
        );
        let resp = try!(self.retry_send(|| {
            self.authed_request(
                Method::Get,
                AcceptType::Regular,
                &url
            )
        }));
        if resp.status == StatusCode::NotFound {
            Ok(false)
        } else if resp.status.is_success() {
            Ok(true)
        } else {
            Err(GithubRequestError::HttpStatus(resp.status))
        }
    }

    fn repo_is_org(
        &self,
        repo: &Repo,
    ) -> Result<bool, GithubRequestError> {
        let url = format!(
            "{}/repos/{}/{}",
            self.host,
            repo.owner,
            repo.repo,
        );
        let resp = try!(self.retry_send(|| {
            self.authed_request(
                Method::Get,
                AcceptType::Regular,
                &url
            )
        }));
        if resp.status.is_success() {
            let repo_desc = try!(json_from_reader::<_, RepositoryDesc>(resp));
            Ok(match &repo_desc.owner.owner_type[..] {
                "User" => false,
                "Organization" => true,
                _ => {
                    warn!(
                        "Unknown owner type: {}",
                        repo_desc.owner.owner_type,
                    );
                    false
                }
            })
        } else {
            Err(GithubRequestError::HttpStatus(resp.status))
        }
    }

    fn get_all_teams_with_write(
        &self,
        repo: &Repo,
    ) -> Result<HashSet<u32>, GithubRequestError> {
        let url = format!(
            "{}/orgs/{}/teams",
            self.host,
            repo.owner,
        );
        let resp = try!(self.retry_send(||{
            self.authed_request(
                Method::Get,
                AcceptType::Regular,
                &url
            )
        }));
        if resp.status.is_success() {
            let all_teams = try!(json_from_reader::<_, Vec<TeamDesc>>(resp));
            let mut writing_teams = HashSet::new();
            for team in all_teams {
                let url = format!(
                    "{}/teams/{}/repos/{}/{}",
                    self.host,
                    team.id,
                    repo.owner,
                    repo.repo
                );
                let resp = try!(self.retry_send(|| {
                    self.authed_request(
                        Method::Get,
                        AcceptType::Repository,
                        &url
                    )
                }));
                let team_repo = try!(json_from_reader::<_, TeamRepoDesc>(
                    resp
                ));
                if team_repo.permissions.admin || team_repo.permissions.push {
                    writing_teams.insert(team.id);
                }
            }
            Ok(writing_teams)
        } else {
            Err(GithubRequestError::HttpStatus(resp.status))
        }
    }
    fn authed_request<'a, U: IntoUrl>(
        &'a self,
        method: Method,
        accept_type: AcceptType,
        url: U
    ) -> RequestBuilder<'a> {
        let mut headers = Headers::new();
        let accept_type: &'static [u8] = match accept_type {
            AcceptType::Regular => b"application/vnd.github.v3+json",
            AcceptType::Repository =>
                b"application/vnd.github.v3.repository+json",
        };
        headers.set_raw(
            "Accept",
            vec![accept_type.to_vec()],
        );
        headers.set_raw("Authorization", vec![self.authorization.clone()]);
        headers.set_raw("User-Agent", vec![
            b"aelita/0.1 (https://github.com/AelitaBot/aelita)".to_vec()
        ]);
        self.client.request(method, url)
            .headers(headers)
    }
    fn retry_send<'a, T: FnMut() -> RequestBuilder<'a>>(
        &self,
        mut f: T,
    ) -> hyper::error::Result<hyper::client::Response> {
        const INIT_RETRY_DELAY_MS: u64 = 500;
        const MAX_RETRY_DELAY_SEC: u64 = 60;
        const MIN_RATE_LIMIT_SEC: u64 = 60;
        let mut rate_limit_until = self.rate_limit_until.lock()
            .expect("Rate limit acquire");
        if let Some(ref rate_limit_until) = *rate_limit_until {
            let duration = time::SystemTime::now()
                .duration_since(*rate_limit_until)
                .unwrap_or(time::Duration::new(MIN_RATE_LIMIT_SEC, 0));
            thread::sleep(duration);
        }
        *rate_limit_until = None;
        let mut delay = time::Duration::from_millis(INIT_RETRY_DELAY_MS);
        let mut result = f().send();
        loop {
            result = if let Err(ref e) = result {
                if delay.as_secs() > MAX_RETRY_DELAY_SEC {
                    warn!("HTTP send failed and gave up: {:?}", e);
                    break;
                } else {
                    warn!("HTTP send failed (retry in {:?}): {:?}", delay, e);
                    thread::sleep(delay);
                    delay = delay * 2;
                    f().send()
                }
            } else {
                break;
            }
        }
        if let Ok(ref result) = result {
            let r = result.headers.get_raw("X-RateLimit-Remaining");
            let delayed = if let Some(r) = r {
                let remaining =
                    String::from_utf8_lossy(&r[0])
                    .parse::<u64>()
                    .expect("Github to give me a number here");
                remaining < 2 // let the other thread send a request
            } else {
                false
            };
            if delayed {
                let until = result.headers.get_raw("X-RateLimit-Reset");
                if let Some(until) = until {
                    let until = String::from_utf8_lossy(&until[0])
                        .parse::<u64>()
                        .unwrap_or(0);
                    let until_unix = time::Duration::new(until, 0);
                    let now_unix = (
                        time::SystemTime::now()
                            .duration_since(time::UNIX_EPOCH)
                    ).expect("It's after the unix epoch");
                    let until_delay = if now_unix > until_unix {
                        time::Duration::new(MIN_RATE_LIMIT_SEC, 0)
                    } else {
                        until_unix - now_unix
                    };
                    *rate_limit_until = Some(
                        time::SystemTime::now() + until_delay
                    );
                }
            }
        }
        result
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
