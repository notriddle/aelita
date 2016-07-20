// This file is released under the same terms as Rust itself.

mod cache;

use crossbeam;
use hyper;
use hyper::Url;
use hyper::buffer::BufReader;
use hyper::client::{Client, IntoUrl, RequestBuilder};
use hyper::header::{Headers, UserAgent};
use hyper::method::Method;
use hyper::net::{HttpListener, NetworkListener, NetworkStream};
use hyper::server::{Request, Response};
use hyper::status::StatusCode;
use pipeline::{self, PipelineId};
use serde_json::{
    self,
    from_reader as json_from_reader,
    from_slice as json_from_slice,
    to_vec as json_to_vec
};
use std;
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::convert::AsRef;
use std::fmt::{self, Display, Formatter};
use std::io::BufWriter;
use std::iter;
use std::num::ParseIntError;
use std::path::Path;
use std::str::FromStr;
use std::sync::Mutex;
use std::sync::mpsc::{Sender, Receiver};
use ui::{self, comments};
use ui::github::cache::{Cache, SqliteCache};
use util::USER_AGENT;
use util::github_headers;
use util::rate_limited_client::RateLimiter;
use vcs::git::{Commit, Remote};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TeamId(pub u32);

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct Repo {
    pub owner: String,
    pub repo: String,
}

#[derive(Debug)]
struct RepoConfig {
    pipeline_id: PipelineId,
    try_pipeline_id: Option<PipelineId>,
}

pub struct Worker {
    listen: String,
    host: String,
    repos: HashMap<Repo, RepoConfig>,
    authorization: Vec<u8>,
    client: Client,
    rate_limiter: RateLimiter,
    user_ident: String,
    secret: String,
    cache: Mutex<cache::SqliteCache>,
}

impl Worker {
    pub fn new<Q: AsRef<Path>>(
        listen: String,
        host: String,
        token: String,
        user: String,
        secret: String,
        cache_path: Q,
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
            client: Client::new(),
            rate_limiter: RateLimiter::new(),
            secret: secret,
            cache: Mutex::new(
                SqliteCache::open(cache_path).expect("FS to work")
            ),
        }
    }
    pub fn add_project(
        &mut self,
        pipeline_id: PipelineId,
        try_pipeline_id: Option<PipelineId>,
        repo: Repo,
    ) {
        if self.cache.get_mut().unwrap().is_org(pipeline_id).is_none() {
            if self.repo_is_org(&repo).expect("Github to respond") {
                let teams = self.get_all_teams_with_write(&repo)
                    .expect("Github to be up");
                let teams = teams.iter().cloned();
                let cache = self.cache.get_mut().unwrap();
                cache.set_teams_with_write(pipeline_id, teams);
                cache.set_is_org(pipeline_id, true);
            } else {
                let cache = self.cache.get_mut().unwrap();
                cache.set_is_org(pipeline_id, false);
            }
        }
        self.repos.insert(repo, RepoConfig{
            pipeline_id: pipeline_id,
            try_pipeline_id: try_pipeline_id,
        });
    }
}

// JSON API structs
#[derive(Serialize, Deserialize)]
struct IssueCommentPullRequest {
    html_url: String,
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
    permissions: Option<TeamRepoPermissions>,
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
    state: String,
    number: u32,
    head: PrBranchDesc,
    html_url: String,
    title: String,
}
#[derive(Deserialize, Serialize)]
struct PullRequestDesc {
    action: String,
    pull_request: PrDesc,
    repository: RepositoryDesc,
}
#[derive(Deserialize, Serialize)]
struct StatusDesc {
    state: String,
    target_url: Option<String>,
    description: String,
    context: String,
}
#[derive(Deserialize, Serialize)]
struct TeamAddDesc {
    repository: RepositoryDesc,
}

impl pipeline::Worker<
    ui::Event<Pr>,
    ui::Message<Pr>,
> for Worker {
    fn run(
        &self,
        recv_msg: Receiver<ui::Message<Pr>>,
        mut send_event: Sender<ui::Event<Pr>>
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
        send_event: Sender<ui::Event<Pr>>,
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
        send_event: &Sender<ui::Event<Pr>>
    ) {
        let head = github_headers::parse(&mut req, self.secret.as_bytes());
        let (x_github_event, body) = match head {
            Some(head) => head,
            None => return,
        };
        match &x_github_event[..] {
            b"issue_comment" => {
                if let Ok(desc) = json_from_slice::<CommentDesc>(&body) {
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
            b"pull_request" => {
                if let Ok(desc) = json_from_slice::<PullRequestDesc>(&body) {
                    info!(
                        "Got PR message for #{}: {}",
                        desc.pull_request.number,
                        desc.action,
                    );
                    *res.status_mut() = StatusCode::NoContent;
                    if let Err(e) = res.send(&[]) {
                        warn!("Failed to send response to Github PR: {:?}", e);
                    }
                    let repo = Repo{
                        owner: desc.repository.owner.login,
                        repo: desc.repository.name,
                    };
                    let pr = Pr(desc.pull_request.number);
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
                    let commit = match Commit::from_str(
                        &desc.pull_request.head.sha
                    ) {
                        Ok(commit) => commit,
                        Err(e) => {
                            warn!("Invalid commit sha with PR event: {:?}", e);
                            return;
                        }
                    };
                    if let Some(pipeline_id) = repo_config.try_pipeline_id {
                        self.handle_pr_update(
                            &desc.action[..],
                            send_event,
                            pipeline_id,
                            commit,
                            pr,
                            desc.pull_request.title.clone(),
                            desc.pull_request.html_url.clone(),
                        );
                    }
                    self.handle_pr_update(
                        &desc.action[..],
                        send_event,
                        repo_config.pipeline_id,
                        commit,
                        pr,
                        desc.pull_request.title,
                        desc.pull_request.html_url,
                    );
                } else {
                    warn!("Got invalid PR message");
                    *res.status_mut() = StatusCode::BadRequest;
                    if let Err(e) = res.send(&[]) {
                        warn!("Failed to send response to bad PR: {:?}", e);
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
            b"team_add" => {
                if let Ok(desc) = json_from_slice::<TeamAddDesc>(&body) {
                    info!("Got team add event");
                    *res.status_mut() = StatusCode::NoContent;
                    if let Err(e) = res.send(&[]) {
                        warn!(
                            "Failed to send response to Github team add: {:?}",
                            e,
                        );
                    }
                    let repo = Repo{
                        owner: desc.repository.owner.login,
                        repo: desc.repository.name,
                    };
                    let repo_config = match self.repos.get(&repo) {
                        Some(repo_config) => repo_config,
                        None => {
                            warn!("Got team add event for nonexistant repo");
                            return;
                        }
                    };
                    let mut cache = self.cache.lock().unwrap();
                    let teams = match self.get_all_teams_with_write(&repo) {
                        Ok(t) => t,
                        Err(e) => {
                            warn!("Failed to refresh teams: {:?}", e);
                            return;
                        }
                    };
                    cache.set_teams_with_write(
                        repo_config.pipeline_id,
                        teams.iter().cloned(),
                    );
                } else {
                    warn!("Got invalid team add event");
                    *res.status_mut() = StatusCode::BadRequest;
                    if let Err(e) = res.send(&[]) {
                        warn!(
                            "Failed to send response to bad Github team: {:?}",
                            e,
                        );
                    }
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

    fn handle_pr_update(
        &self,
        action: &str,
        send_event: &Sender<ui::Event<Pr>>,
        pipeline_id: PipelineId,
        commit: Commit,
        pr: Pr,
        title: String,
        html_url: String,
    ) {
        let event = match action {
            "closed" => Some(ui::Event::Closed(
                pipeline_id,
                pr,
            )),
            "opened" | "reopened" => Some(ui::Event::Opened(
                pipeline_id,
                pr,
                commit,
                title,
                Url::parse(&html_url).unwrap(),
            )),
            "synchronize" | "edited" => Some(ui::Event::Changed(
                pipeline_id,
                pr,
                commit,
                title,
                Url::parse(&html_url).unwrap(),
            )),
            _ => None,
        };
        if let Some(event) = event {
            send_event.send(event).expect("Pipeline to be there");
        }
    }

    fn handle_pr_comment(
        &self,
        send_event: &Sender<ui::Event<Pr>>,
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
                repo_config,
                &pr,
            );
        } else {
            info!("Pull request comment is not a command");
        }
    }

    fn handle_comment_command(
        &self,
        send_event: &Sender<ui::Event<Pr>>,
        command: comments::Command<Commit>,
        issue: &IssueCommentIssue,
        repo_config: &RepoConfig,
        pr: &Pr,
    ) {
        match command {
            comments::Command::Approved(user, commit) => {
                self.handle_approved_pr(
                    repo_config.pipeline_id,
                    send_event,
                    issue,
                    pr,
                    user,
                    commit,
                );
            }
            comments::Command::Canceled => {
                self.handle_canceled_pr(
                    repo_config.pipeline_id,
                    send_event,
                    pr,
                );
            }
            comments::Command::TryApproved(user, commit) => {
                if let Some(try_pipeline_id) = repo_config.try_pipeline_id {
                    self.handle_approved_pr(
                        try_pipeline_id,
                        send_event,
                        issue,
                        pr,
                        user,
                        commit,
                    );
                }
            }
            comments::Command::TryCanceled => {
                if let Some(try_pipeline_id) = repo_config.try_pipeline_id {
                    self.handle_canceled_pr(
                        try_pipeline_id,
                        send_event,
                        pr,
                    );
                }
            }
        }
    }

    fn handle_approved_pr(
        &self,
        pipeline_id: PipelineId,
        send_event: &Sender<ui::Event<Pr>>,
        issue: &IssueCommentIssue,
        pr: &Pr,
        user: &str,
        commit: Option<Commit>,
    ) {
        let message = format!(
            "{}\n\nMerge #{} a=@{} r=@{}\n{}\n\n{}",
            issue.title,
            pr,
            issue.user.login,
            user,
            iter::repeat('_').take(72).collect::<String>(),
            issue.body,
        );
        send_event.send(ui::Event::Approved(
            pipeline_id,
            *pr,
            commit,
            message,
        )).expect("PR Approved: Pipeline error");
    }

    fn handle_canceled_pr(
        &self,
        pipeline_id: PipelineId,
        send_event: &Sender<ui::Event<Pr>>,
        pr: &Pr,
    ) {
        send_event.send(ui::Event::Canceled(
            pipeline_id,
            *pr,
        )).expect("PR Canceled: Pipeline error");
    }

    fn handle_message(
        &self,
        msg: ui::Message<Pr>,
        _: &mut Sender<ui::Event<Pr>>,
    ) {
        match msg {
            ui::Message::SendResult(pipeline_id, pr, status) => {
                let result = self.send_result_to_pr(pipeline_id, pr, &status);
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
        let mut cache = self.cache.lock().unwrap();
        if cache.is_org(repo_config.pipeline_id) == Some(true) {
            info!("Using teams permission check");
            let teams = cache.teams_with_write(repo_config.pipeline_id);
            let mut allowed = false;
            for team in teams {
                if try!(self.user_is_member_of(user, team)) {
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

    fn send_result_to_pr(
        &self,
        pipeline_id: PipelineId,
        pr: Pr,
        status: &ui::Status<Pr>,
    ) -> Result<(), GithubRequestError> {
        let mut repo = None;
        let mut is_try = false;
        for (r, c) in &self.repos {
            if c.pipeline_id == pipeline_id {
                repo = Some(r);
            } else if c.try_pipeline_id == Some(pipeline_id) {
                repo = Some(r);
                is_try = true;
            }
        }
        let repo = match repo {
            Some(repo) => repo,
            None => {
                return Err(GithubRequestError::Pipeline(pipeline_id));
            }
        };
        let comment_body = match *status {
            ui::Status::Approved(_) => None,
            ui::Status::StartingBuild(_, _) => None,
            ui::Status::Testing(_, _, _) => None,
            ui::Status::Success(_, _, ref url) => Some({
                if let Some(ref url) = *url {
                    Cow::Owned(format!(":+1: [Build succeeded]({})", url))
                } else {
                    Cow::Borrowed(":+1: Build succeeded")
                }
            }),
            ui::Status::Failure(_, _, ref url) => Some({
                if let Some(ref url) = *url {
                    Cow::Owned(format!(":-1: [Build failed]({})", url))
                } else {
                    Cow::Borrowed(":-1: Build failed")
                }
            }),
            ui::Status::Unmergeable(_) => Some(Cow::Borrowed(
                ":x: Merge conflict!"
            )),
            ui::Status::Unmoveable(_, _) => Some(Cow::Borrowed(
                ":scream: Internal error while fast-forward master"
            )),
            ui::Status::Invalidated => Some(Cow::Borrowed(
                ":not_good: New commits added"
            )),
            ui::Status::NoCommit => Some(Cow::Borrowed(
                ":scream: Internal error: no commit found for PR"
            )),
            ui::Status::Completed(_, _) => None,
        };
        let context = if is_try {
            "continuous-integration/aelita/try".to_owned()
        } else {
            "continuous-integration/aelita".to_owned()
        };
        let status = match *status {
            ui::Status::Approved(ref pull_commit) => Some((
                pull_commit,
                None,
                StatusDesc {
                    state: "pending".to_owned(),
                    target_url: None,
                    description: format!(
                        "Approved {}",
                        pull_commit,
                    ),
                    context: context,
                }
            )),
            ui::Status::StartingBuild(
                ref pull_commit,
                ref merge_commit,
            ) => Some((
                pull_commit,
                Some(merge_commit),
                StatusDesc {
                    state: "pending".to_owned(),
                    target_url: None,
                    description: format!(
                        "Testing {} with merge commit {}",
                        &pull_commit.to_string()[0..5],
                        &merge_commit.to_string()[0..5],
                    ),
                    context: context,
                }
            )),
            ui::Status::Testing(
                ref pull_commit,
                ref merge_commit,
                ref url,
            ) => Some((
                pull_commit,
                Some(merge_commit),
                StatusDesc {
                    state: "pending".to_owned(),
                    target_url: url.as_ref().map(ToString::to_string),
                    description: format!(
                        "Testing {} with merge commit {}",
                        &pull_commit.to_string()[0..5],
                        &merge_commit.to_string()[0..5],
                    ),
                    context: context,
                }
            )),
            ui::Status::Success(
                ref pull_commit,
                ref merge_commit,
                ref url,
            ) => Some((
                pull_commit,
                Some(merge_commit),
                StatusDesc {
                    state: "success".to_owned(),
                    target_url: url.as_ref().map(ToString::to_string),
                    description: "Tests passed".to_owned(),
                    context: context,
                }
            )),
            ui::Status::Failure(
                ref pull_commit,
                ref merge_commit, 
                ref url,
            ) => Some((
                pull_commit,
                Some(merge_commit),
                StatusDesc {
                    state: "failure".to_owned(),
                    target_url: url.as_ref().map(ToString::to_string),
                    description: "Tests failed".to_owned(),
                    context: context,
                }
            )),
            ui::Status::Unmergeable(
                ref pull_commit,
            ) => Some((
                pull_commit,
                None,
                StatusDesc {
                    state: "failure".to_owned(),
                    target_url: None,
                    description: "Merge failed".to_owned(),
                    context: context,
                }
            )),
            ui::Status::Unmoveable(
                ref pull_commit,
                ref merge_commit,
            ) => Some((
                pull_commit,
                Some(merge_commit),
                StatusDesc {
                    state: "error".to_owned(),
                    target_url: None,
                    description: "Merge failed".to_owned(),
                    context: context,
                }
            )),
            ui::Status::Invalidated | ui::Status::NoCommit => None,
            ui::Status::Completed(_, _) => None,
        };
        if let Some(comment_body) = comment_body {
            let url = format!(
                "{}/repos/{}/{}/issues/{}/comments",
                self.host,
                repo.owner,
                repo.repo,
                pr
            );
            let comment = try!(json_to_vec(&PostCommentComment{
                body: comment_body.into_owned(),
            }));
            let resp = try!(self.rate_limiter.retry_send(|| {
                self.authed_request(Method::Post, AcceptType::Regular, &url)
                    .body(&*comment)
            }));
            if !resp.status.is_success() {
                return Err(GithubRequestError::HttpStatus(resp.status))
            }
        }
        if let Some(status) = status {
            let (pull_commit, merge_commit, status_body) = status;
            let url = format!(
                "{}/repos/{}/{}/statuses/{}",
                self.host,
                repo.owner,
                repo.repo,
                pull_commit
            );
            let status_body = try!(json_to_vec(&status_body));
            let resp = try!(self.rate_limiter.retry_send(|| {
                self.authed_request(Method::Post, AcceptType::Regular, &url)
                    .body(&*status_body)
            }));
            if !resp.status.is_success() {
                return Err(GithubRequestError::HttpStatus(resp.status))
            }
            if let Some(merge_commit) = merge_commit {
                let url = format!(
                    "{}/repos/{}/{}/statuses/{}",
                    self.host,
                    repo.owner,
                    repo.repo,
                    merge_commit
                );
                let resp = try!(self.rate_limiter.retry_send(|| {
                    self.authed_request(
                        Method::Post,
                        AcceptType::Regular,
                        &url
                    ).body(&*status_body)
                }));
                if !resp.status.is_success() {
                    return Err(GithubRequestError::HttpStatus(resp.status))
                }
            }
        }
        Ok(())
    }

    fn user_is_member_of(
        &self,
        user: &str,
        team: TeamId,
    ) -> Result<bool, GithubRequestError> {
        let url = format!(
            "{}/teams/{}/members/{}",
            self.host,
            team.0,
            user,
        );
        let resp = try!(self.rate_limiter.retry_send(|| {
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
        let resp = try!(self.rate_limiter.retry_send(|| {
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
        let resp = try!(self.rate_limiter.retry_send(|| {
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
    ) -> Result<HashSet<TeamId>, GithubRequestError> {
        let url = format!(
            "{}/orgs/{}/teams",
            self.host,
            repo.owner,
        );
        let resp = try!(self.rate_limiter.retry_send(||{
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
                let resp = try!(self.rate_limiter.retry_send(|| {
                    self.authed_request(
                        Method::Get,
                        AcceptType::Repository,
                        &url
                    )
                }));
                let team_repo = try!(json_from_reader::<_, TeamRepoDesc>(
                    resp
                ));
                if let Some(ref permissions) = team_repo.permissions {
                    if permissions.admin || permissions.push {
                        writing_teams.insert(TeamId(team.id));
                    }
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
        headers.set(UserAgent(USER_AGENT.to_owned()));
        self.client.request(method, url)
            .headers(headers)
    }
}

enum AcceptType {
    Regular,
    Repository,
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
    type C = Commit;
    fn remote(&self) -> <Commit as ::vcs::Commit>::Remote {
        Remote(format!("pull/{}/head", self.0))
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
