// This file is released under the same terms as Rust itself.

use hyper;
use hyper::client::{Client, IntoUrl, RequestBuilder};
use hyper::header::{Headers, UserAgent};
use hyper::method::Method;
use hyper::status::StatusCode;
use pipeline::{self, PipelineId};
use serde_json::{
    self,
    from_reader as json_from_reader,
    to_vec as json_to_vec
};
use std;
use std::convert::From;
use std::str::FromStr;
use std::sync::mpsc::{Sender, Receiver};
use util::USER_AGENT;
use util::rate_limited_client::RateLimiter;
use vcs;
use vcs::git::Commit;

pub trait PipelinesConfig: Send + Sync + 'static {
    fn repo_by_pipeline(&self, PipelineId) -> Option<Repo>;
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct Repo {
    pub owner: String,
    pub repo: String,
    pub master_branch: String,
    pub staging_branch: String,
    pub push_to_master: bool,
}

pub struct Worker {
    pipelines: Box<PipelinesConfig>,
    host: String,
    client: Client,
    authorization: Vec<u8>,
    rate_limiter: RateLimiter,
}

impl Worker {
    pub fn new(
        host: String,
        token: String,
        pipelines: Box<PipelinesConfig>
    ) -> Worker {
        let mut authorization: Vec<u8> = b"token ".to_vec();
        authorization.extend(token.bytes());
        Worker{
            pipelines: pipelines,
            host: host,
            client: Client::default(),
            authorization: authorization,
            rate_limiter: RateLimiter::new(),
        }
    }
}

impl pipeline::Worker<vcs::Event<Commit>, vcs::Message<Commit>> for Worker {
    fn run(
        &self,
        recv_msg: Receiver<vcs::Message<Commit>>,
        mut send_event: Sender<vcs::Event<Commit>>
    ) {
        loop {
            self.handle_message(
                recv_msg.recv().expect("Pipeline went away"),
                &mut send_event,
            );
        }
    }
}

impl Worker {
    fn handle_message(
        &self,
        msg: vcs::Message<Commit>,
        send_event: &mut Sender<vcs::Event<Commit>>
    ) {
        match msg {
            vcs::Message::MergeToStaging(
                pipeline_id, pull_commit, message, _
            ) => {
                match self.merge_to_staging(
                    pipeline_id, pull_commit, message
                ) {
                    Ok(merge_commit) => {
                        send_event.send(vcs::Event::MergedToStaging(
                            pipeline_id,
                            pull_commit,
                            merge_commit,
                        )).expect("Pipeline gone merge to staging");
                    },
                    Err(e) => {
                        warn!("Failed to merge to staging: {:?}", e);
                        send_event.send(vcs::Event::FailedMergeToStaging(
                            pipeline_id,
                            pull_commit,
                        )).expect("Pipeline gone merge to staging error");
                    }
                }
            }
            vcs::Message::MoveStagingToMaster(pipeline_id, merge_commit) => {
                match self.move_to_master(pipeline_id, merge_commit) {
                    Ok(()) => {
                        send_event.send(vcs::Event::MovedToMaster(
                            pipeline_id,
                            merge_commit,
                        )).expect("Pipeline gone move to master");
                    },
                    Err(e) => {
                        warn!("Failed to move to master: {:?}", e);
                        send_event.send(vcs::Event::FailedMoveToMaster(
                            pipeline_id,
                            merge_commit,
                        )).expect("Pipeline gone move to master error");
                    }
                }
            }
        }
    }
    fn move_to_master(
        &self,
        pipeline_id: PipelineId,
        merge_commit: Commit,
    ) -> Result<(), GithubRequestError> {
        let repo = match self.pipelines.repo_by_pipeline(pipeline_id) {
            Some(repo) => repo,
            None => return Err(GithubRequestError::Pipeline(pipeline_id)),
        };
        let url = format!(
            "{}/repos/{}/{}/git/refs/heads/{}",
            self.host,
            repo.owner,
            repo.repo,
            repo.master_branch
        );
        debug!("Set master SHA: {}", url);
        #[derive(Serialize)]
        struct RefUpdateDesc {
            force: bool,
            sha: String,
        }
        let update_desc = try!(json_to_vec(&RefUpdateDesc {
            force: false,
            sha: merge_commit.to_string(),
        }));
        let resp = try!(self.rate_limiter.retry_send(|| {
            self.authed_request(Method::Patch, &url)
                .body(&*update_desc)
        }));
        if !resp.status.is_success() {
            return Err(GithubRequestError::HttpStatus(resp.status));
        }
        Ok(())
    }
    fn merge_to_staging(
        &self,
        pipeline_id: PipelineId,
        pull_commit: Commit,
        message: String,
    ) -> Result<Commit, GithubRequestError> {
        let repo = match self.pipelines.repo_by_pipeline(pipeline_id) {
            Some(repo) => repo,
            None => return Err(GithubRequestError::Pipeline(pipeline_id)),
        };
        // Step 1: get the contents of master.
        let url = format!(
            "{}/repos/{}/{}/git/refs/heads/{}",
            self.host,
            repo.owner,
            repo.repo,
            repo.master_branch
        );
        debug!("Get master SHA: {}", url);
        #[derive(Deserialize)]
        struct ObjectDesc {
            sha: String,
        }
        #[derive(Deserialize)]
        struct RefDesc {
            object: ObjectDesc,
        }
        let resp = try!(self.rate_limiter.retry_send(|| {
            self.authed_request(Method::Get, &url)
        }));
        if !resp.status.is_success() {
            return Err(GithubRequestError::HttpStatus(resp.status));
        }
        let resp_desc: RefDesc = try!(json_from_reader(resp));
        let master_sha = resp_desc.object.sha;
        // Step 2: reset staging to the contents of master.
        // Do it in a single step if no rewinding is needed, but we may
        // need to rewind.
        let url = format!(
            "{}/repos/{}/{}/git/refs/heads/{}",
            self.host,
            repo.owner,
            repo.repo,
            repo.staging_branch
        );
        let resp = try!(self.rate_limiter.retry_send(|| {
            self.authed_request(Method::Get, &url)
        }));
        if !resp.status.is_success() {
            return Err(GithubRequestError::HttpStatus(resp.status));
        }
        let resp_desc: RefDesc = try!(json_from_reader(resp));
        let init_staging_sha = resp_desc.object.sha;
        debug!("Staging sha is: {}", init_staging_sha);
        if init_staging_sha != master_sha {
            debug!("Set staging SHA: {}", url);
            #[derive(Serialize)]
            struct RefUpdateDesc {
                force: bool,
                sha: String,
            }
            let update_desc = try!(json_to_vec(&RefUpdateDesc {
                force: true,
                sha: master_sha,
            }));
            let resp = try!(self.rate_limiter.retry_send(|| {
                self.authed_request(Method::Patch, &url)
                    .body(&*update_desc)
            }));
            if !resp.status.is_success() {
                return Err(GithubRequestError::HttpStatus(resp.status));
            }
        }
        // Step 3: merge the pull request into master.
        let url = format!(
            "{}/repos/{}/{}/merges",
            self.host,
            repo.owner,
            repo.repo
        );
        debug!("Merge PR into staging: {}", url);
        #[derive(Serialize)]
        struct MergeDesc {
            base: String,
            head: String,
            commit_message: String,
        }
        #[derive(Deserialize)]
        struct MergeResultDesc {
            sha: String,
        }
        let merge_desc = try!(json_to_vec(&MergeDesc {
            base: repo.staging_branch.clone(),
            head: pull_commit.to_string(),
            commit_message: message,
        }));
        let resp = try!(self.rate_limiter.retry_send(|| {
            self.authed_request(Method::Post, &url)
                .body(&*merge_desc)
        }));
        if !resp.status.is_success() {
            return Err(GithubRequestError::HttpStatus(resp.status));
        }
        let resp_desc: MergeResultDesc = try!(json_from_reader(resp));
        Ok(try!(Commit::from_str(&resp_desc.sha)))
    }
    fn authed_request<'a, U: IntoUrl>(
        &'a self,
        method: Method,
        url: U
    ) -> RequestBuilder<'a> {
        let mut headers = Headers::new();
        let accept_type = b"application/vnd.github.v3+json";
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