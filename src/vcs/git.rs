// This file is released under the same terms as Rust itself.

use pipeline::{self, PipelineId};
use std;
use std::collections::HashMap;
use std::convert::{From, Into};
use std::fmt::{self, Debug, Display, Formatter};
use std::fs::File;
use std::io::Read;
use std::num::ParseIntError;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::str::FromStr;
use std::sync::mpsc::{Sender, Receiver};
use vcs;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct Repo {
    pub path: PathBuf,
    pub origin: String,
    pub master_branch: String,
    pub staging_branch: String,
}

pub struct Worker {
    repos: HashMap<PipelineId, Repo>,
    executable: String,
}

impl Worker {
    pub fn new(
        executable: String,
    ) -> Worker {
        Worker{
            repos: HashMap::new(),
            executable: executable,
        }
    }
    pub fn add_pipeline(&mut self, pipeline_id: PipelineId, repo: Repo) {
        self.repos.insert(pipeline_id, repo);
    }
}

impl pipeline::Worker<vcs::Event<Commit>, vcs::Message<Commit>> for Worker {
    fn run(
        &mut self,
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

macro_rules! try_cmd {
    ($e:expr) => ({
        let cmd = try!($e);
        if !cmd.status.success() {
            return Err(GitError::Cli(
                cmd.status,
                String::from_utf8_lossy(&cmd.stderr).into_owned()
            ));
        }
        cmd
    })
}

impl Worker {
    fn handle_message(
        &self,
        msg: vcs::Message<Commit>,
        send_event: &mut Sender<vcs::Event<Commit>>
    ) {
        match msg {
            vcs::Message::MergeToStaging(
                pipeline_id, pull_commit, message, remote
            ) => {
                let repo = match self.repos.get(&pipeline_id) {
                    Some(repo) => repo,
                    None => {
                        warn!("Got wrong pipeline ID {:?}", pipeline_id);
                        return;
                    }
                };
                info!("Merging {} ...", pull_commit);
                match self.merge_to_staging(
                    repo, pull_commit, &message, &remote
                ) {
                    Err(e) => {
                        warn!(
                            "Failed to merge {} to staging: {:?}",
                            pull_commit,
                            e
                        );
                        send_event.send(vcs::Event::FailedMergeToStaging(
                            pipeline_id,
                            pull_commit,
                        )).expect("Pipeline gone merge to staging error");
                    }
                    Ok(merge_commit) => {
                        info!("Merged {} to {}", pull_commit, merge_commit);
                        send_event.send(vcs::Event::MergedToStaging(
                            pipeline_id,
                            pull_commit,
                            merge_commit,
                        )).expect("Pipeline gone merge to staging");
                    }
                }
            }
            vcs::Message::MoveStagingToMaster(pipeline_id, merge_commit) => {
                let repo = match self.repos.get(&pipeline_id) {
                    Some(repo) => repo,
                    None => {
                        warn!("Got wrong pipeline ID {:?}", pipeline_id);
                        return;
                    }
                };
                info!("Moving {} ...", merge_commit);
                match self.move_staging_to_master(repo, merge_commit) {
                    Err(e) => {
                        warn!(
                            "Failed to move {} to master: {:?}",
                            merge_commit,
                            e
                        );
                        send_event.send(vcs::Event::FailedMoveToMaster(
                            pipeline_id,
                            merge_commit,
                        )).expect("Pipeline gone move to master error");
                    }
                    Ok(()) => {
                        info!("Moved {} to master", merge_commit);
                        send_event.send(vcs::Event::MovedToMaster(
                            pipeline_id,
                            merge_commit,
                        )).expect("Pipeline gone move to master");
                    }
                }
            }
        }
    }
    fn merge_to_staging(
        &self,
        repo: &Repo,
        pull_commit: Commit,
        message: &str,
        remote: &str,
    ) -> Result<Commit, GitError> {
        try!(self.setup_dir(repo));
        try_cmd!(Command::new(&self.executable)
            .arg("-C")
            .arg(&repo.path)
            .arg("fetch")
            .arg("origin")
            .arg(&repo.master_branch)
            .arg(remote)
            .output());
        try_cmd!(Command::new(&self.executable)
            .arg("-C")
            .arg(&repo.path)
            .arg("checkout")
            .arg(format!("origin/{}", repo.master_branch))
            .output());
        try_cmd!(Command::new(&self.executable)
            .arg("-C")
            .arg(&repo.path)
            .arg("branch")
            .arg("-f")
            .arg(&repo.staging_branch)
            .arg(format!("origin/{}", repo.master_branch))
            .output());
        try_cmd!(Command::new(&self.executable)
            .arg("-C")
            .arg(&repo.path)
            .arg("checkout")
            .arg(&repo.staging_branch)
            .output());
        try_cmd!(Command::new(&self.executable)
            .arg("-C")
            .arg(&repo.path)
            .arg("merge")
            .arg("--no-ff")
            .arg("-m")
            .arg(message)
            .arg(&pull_commit.to_string())
            .output());
        try_cmd!(Command::new(&self.executable)
            .arg("-C")
            .arg(&repo.path)
            .arg("push")
            .arg("-f")
            .arg("origin")
            .arg(&repo.staging_branch)
            .output());
        let mut commit_string = String::new();
        try!(try!(File::open(
            Path::new(&repo.path)
                .join(".git/refs/heads/")
                .join(&repo.staging_branch)
        )).read_to_string(&mut commit_string));
        commit_string = commit_string.replace("\n", "").replace("\r", "");
        Commit::from_str(&commit_string).map_err(|e| e.into())
    }
    fn move_staging_to_master(
        &self,
        repo: &Repo,
        merge_commit: Commit,
    ) -> Result<(), GitError> {
        try!(self.setup_dir(repo));
        try_cmd!(Command::new(&self.executable)
            .arg("-C")
            .arg(&repo.path)
            .arg("push")
            .arg("-f")
            .arg("origin")
            .arg(format!("{}:{}", merge_commit, &repo.master_branch))
            .output());
        Ok(())
    }
    fn setup_dir(&self, repo: &Repo) -> Result<(), GitError> {
        if !repo.path.exists() {
            try_cmd!(Command::new(&self.executable)
                .arg("init")
                .arg(&repo.path)
                .output());
            try_cmd!(Command::new(&self.executable)
                .arg("-C")
                .arg(&repo.path)
                .arg("remote")
                .arg("add")
                .arg("origin")
                .arg(&repo.origin)
                .output());
        } else {
            try!(Command::new(&self.executable)
                .arg("-C")
                .arg(&repo.path)
                .arg("merge")
                .arg("--abort")
                .output());
        }
        Ok(())
    }
}

quick_error! {
    #[derive(Debug)]
    pub enum GitError {
        Int(err: std::num::ParseIntError) {
            cause(err)
            from()
        }
        Io(err: std::io::Error) {
            cause(err)
            from()
        }
        Cli(status: std::process::ExitStatus, output: String) {}
    }
}

// A git commit is a SHA1 sum. A SHA1 sum is a 160-bit number.
#[derive(Copy, Clone, Deserialize, Eq, PartialEq, Serialize)]
pub struct Commit(u64, u64, u32);

impl vcs::Commit for Commit {}

impl Display for Commit {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(f, "{:016x}{:016x}{:08x}", self.0, self.1, self.2)
    }
}

impl Debug for Commit {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(f, "Commit({:016x}{:016x}{:08x})", self.0, self.1, self.2)
    }
}

// It should be easy to get from hexadecimal.

impl FromStr for Commit {
    type Err = ParseIntError;
    fn from_str(mut s: &str) -> Result<Commit, ParseIntError> {
        if s.len() != 40 {
            s = "THIS_IS_NOT_A_NUMBER_BUT_I_CANT_MAKE_PARSEINTERROR_MYSELF";
        }
        let a = try!(u64::from_str_radix(&s[0..16], 16));
        let b = try!(u64::from_str_radix(&s[16..32], 16));
        let c = try!(u32::from_str_radix(&s[32..40], 16));
        Ok(Commit(a, b, c))
    }
}

impl Into<String> for Commit {
    fn into(self) -> String {
        self.to_string()
    }
}
