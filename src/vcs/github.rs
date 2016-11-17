// This file is released under the same terms as Rust itself.

use hyper;
use hyper::header::{self, qitem, Accept};
use hyper::status::StatusCode;
use pipeline::{self, PipelineId};
use rest::{authorization, Authorization, Client, Mime};
use serde_json;
use std;
use std::convert::From;
use std::str::FromStr;
use std::sync::mpsc::{Sender, Receiver};
use util::USER_AGENT;
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
    client: Client<Authorization<authorization::Token>>,
}

impl Worker {
    pub fn new(
        host: String,
        token: String,
        pipelines: Box<PipelinesConfig>
    ) -> Worker {
        Worker{
            pipelines: pipelines,
            client: Client::new(USER_AGENT.to_owned())
                .base(&host)
                .authorization(Authorization(authorization::Token{
                    token: token,
                })),
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

        // If GitHub's UI is also being used, it will also do this. But
        // protected branches rely on a strict happens-before relationship.
        debug!("Set master status (avoid race)");
        let url = format!(
            "/repos/{}/{}/statuses/{}",
            repo.owner,
            repo.repo,
            merge_commit
        );
        #[derive(Serialize)]
        struct StatusDesc {
            state: String,
            target_url: Option<String>,
            description: String,
            context: String,
        }
        let status_body = StatusDesc{
            state: "success".to_owned(),
            target_url: None,
            description: "Tests passed".to_owned(),
            context: "continuous-integration/aelita".to_owned(),
        };
        let resp = try!(
            try!(self.client.post(&url).expect("url").json(&status_body))
                .header(Self::accept())
                .send()
        );
        if !resp.is_success() {
            return Err(GithubRequestError::HttpStatus(resp.http.status))
        }

        debug!("Set master SHA: {}", url);
        let url = format!(
            "/repos/{}/{}/git/refs/heads/{}",
            repo.owner,
            repo.repo,
            repo.master_branch
        );
        #[derive(Serialize)]
        struct RefUpdateDesc {
            force: bool,
            sha: String,
        }
        let update_desc = RefUpdateDesc{
            force: false,
            sha: merge_commit.to_string(),
        };
        let resp = try!(
            try!(
                self.client.patch(&url).expect("valid url")
                    .json(&update_desc)
            )
                .header(Self::accept())
                .send()
        );
        if !resp.is_success() {
            return Err(GithubRequestError::HttpStatus(resp.http.status));
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
            "/repos/{}/{}/git/refs/heads/{}",
            repo.owner,
            repo.repo,
            repo.master_branch
        );
        debug!("Get master SHA: {}", url);
        #[derive(Deserialize, Serialize)]
        struct ObjectDesc {
            sha: String,
        }
        #[derive(Deserialize, Serialize)]
        struct RefDesc {
            object: ObjectDesc,
        }
        let resp = try!(
            self.client.get(&url).expect("valid url")
                .header(Self::accept())
                .send()
        );
        if !resp.is_success() {
            return Err(GithubRequestError::HttpStatus(resp.http.status));
        }
        let resp_desc: RefDesc = try!(resp.json());
        let master_sha = resp_desc.object.sha;
        // Step 2: reset staging to the contents of master.
        // Do it in a single step if no rewinding is needed, but we may
        // need to rewind.
        let url = format!(
            "/repos/{}/{}/git/refs/heads/{}",
            repo.owner,
            repo.repo,
            repo.staging_branch
        );
        let resp = try!(
            self.client.get(&url).expect("valid url")
                .header(Self::accept())
                .send()
        );
        let staging_up_to_date = if resp.is_success() {
            let resp_desc: RefDesc = try!(resp.json());
            let init_staging_sha = resp_desc.object.sha;
            debug!("Staging sha is: {}", init_staging_sha);
            Some(init_staging_sha == master_sha)
        } else {
            None
        };
        match staging_up_to_date {
            Some(false) => {
                debug!("Set staging SHA: {}", url);
                #[derive(Serialize)]
                struct RefUpdateDesc {
                    force: bool,
                    sha: String,
                }
                let update_desc = RefUpdateDesc {
                    force: true,
                    sha: master_sha,
                };
                let resp = try!(
                    try!(
                        self.client.patch(&url).expect("valid url")
                            .json(&update_desc)
                    )
                        .header(Self::accept())
                        .send()
                );
                if !resp.is_success() {
                    return Err(GithubRequestError::HttpStatus(
                        resp.http.status
                    ));
                }
            }
            Some(true) => {}
            None => {
                let url = format!(
                    "/repos/{}/{}/git/refs",
                    repo.owner,
                    repo.repo
                );
                #[derive(Serialize)]
                struct RefCreateDesc {
                    #[serde(rename="ref")]
                    git_ref: String,
                    sha: String,
                }
                let create_desc = RefCreateDesc{
                    git_ref: format!("refs/heads/{}", repo.staging_branch),
                    sha: master_sha,
                };
                let resp = try!(
                    try!(
                        self.client.post(&url).expect("valid url")
                            .json(&create_desc)
                    )
                        .header(Self::accept())
                        .send()
                );
                if !resp.is_success() {
                    return Err(GithubRequestError::HttpStatus(
                        resp.http.status
                    ));
                }
            }
        }
        // Step 3: merge the pull request into master.
        let url = format!(
            "/repos/{}/{}/merges",
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
        #[derive(Deserialize, Serialize)]
        struct MergeResultDesc {
            sha: String,
        }
        let merge_desc = MergeDesc {
            base: repo.staging_branch.clone(),
            head: pull_commit.to_string(),
            commit_message: message,
        };
        let resp = try!(
            try!(self.client.post(&url).expect("valid url").json(&merge_desc))
                .header(Self::accept())
                .send()
        );
        if !resp.is_success() {
            return Err(GithubRequestError::HttpStatus(resp.http.status));
        }
        let resp_desc: MergeResultDesc = try!(resp.json());
        Ok(try!(Commit::from_str(&resp_desc.sha)))
    }
    fn accept() -> Accept {
        let mime: Mime = "application/vnd.github.v3+json"
            .parse().expect("hard-coded mimes to be valid");
        header::Accept(vec![qitem(mime)])
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