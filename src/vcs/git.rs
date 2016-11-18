// This file is released under the same terms as Rust itself.

use pipeline::{self, PipelineId};
use std;
use std::convert::From;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::process::Command;
use std::sync::mpsc::{Sender, Receiver};
use vcs::{self, Commit};

pub trait PipelinesConfig: Send + Sync + 'static {
    fn repo_by_pipeline(&self, PipelineId) -> Option<Repo>;
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct Repo {
    pub path: String,
    pub origin: String,
    pub master_branch: String,
    pub staging_branch: String,
    pub push_to_master: bool,
}

pub struct Worker {
    pipelines: Box<PipelinesConfig>,
    executable: String,
    name: String,
    email: String,
}

impl Worker {
    pub fn new(
        executable: String,
        name: String,
        email: String,
        pipelines: Box<PipelinesConfig>,
    ) -> Worker {
        Worker{
            executable: executable,
            name: name,
            email: email,
            pipelines: pipelines,
        }
    }
}

impl pipeline::Worker<vcs::Event, vcs::Message> for Worker {
    fn run(
        &self,
        recv_msg: Receiver<vcs::Message>,
        mut send_event: Sender<vcs::Event>
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
    ($e:expr, $i:ident, $f:expr) => ({
        let mut $i = $e;
        $f;
        info!("Run command: {:?}", $i);
        let out = try!($i.output());
        if !out.status.success() {
            return Err(GitError::Cli(
                out.status,
                String::from_utf8_lossy(&out.stderr).into_owned()
            ));
        }
        out
    })
}

impl Worker {
    fn handle_message(
        &self,
        msg: vcs::Message,
        send_event: &mut Sender<vcs::Event>
    ) {
        match msg {
            vcs::Message::MergeToStaging(
                pipeline_id, pull_commit, message, remote
            ) => {
                let repo = match self.pipelines.repo_by_pipeline(pipeline_id) {
                    Some(repo) => repo,
                    None => {
                        warn!("Got wrong pipeline ID {:?}", pipeline_id);
                        return;
                    }
                };
                info!("Merging {} ...", pull_commit);
                match self.merge_to_staging(
                    &repo, &pull_commit, &message, &remote.0
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
                let repo = match self.pipelines.repo_by_pipeline(pipeline_id) {
                    Some(repo) => repo,
                    None => {
                        warn!("Got wrong pipeline ID {:?}", pipeline_id);
                        return;
                    }
                };
                info!("Moving {} ...", merge_commit);
                match self.move_staging_to_master(&repo, &merge_commit) {
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
        pull_commit: &Commit,
        message: &str,
        remote: &str,
    ) -> Result<Commit, GitError> {
        try!(self.setup_dir(repo));
        try_cmd!(Command::new(&self.executable), cmd,
        cmd.current_dir(&repo.path)
            .arg("fetch")
            .arg("origin"));
        try_cmd!(Command::new(&self.executable), cmd,
        cmd.current_dir(&repo.path)
            .arg("fetch")
            .arg("origin")
            .arg(&repo.master_branch)
            .arg(remote));
        try_cmd!(Command::new(&self.executable), cmd,
        cmd.current_dir(&repo.path)
            .arg("checkout")
            .arg(format!("origin/{}", repo.master_branch)));
        try_cmd!(Command::new(&self.executable), cmd,
        cmd.current_dir(&repo.path)
            .arg("branch")
            .arg("-f")
            .arg(&repo.staging_branch)
            .arg(format!("origin/{}", repo.master_branch)));
        try_cmd!(Command::new(&self.executable), cmd,
        cmd.current_dir(&repo.path)
            .arg("checkout")
            .arg(&repo.staging_branch));
        try_cmd!(Command::new(&self.executable), cmd,
        cmd.current_dir(&repo.path)
            .arg("merge")
            .arg("--no-ff")
            .arg("-m")
            .arg(message)
            .arg(&pull_commit.to_string()));
        try_cmd!(Command::new(&self.executable), cmd,
        cmd.current_dir(&repo.path)
            .arg("push")
            .arg("-f")
            .arg("origin")
            .arg(&repo.staging_branch));
        let mut commit_string = String::new();
        try!(try!(File::open(
            Path::new(&repo.path)
                .join(".git/refs/heads/")
                .join(&repo.staging_branch)
        )).read_to_string(&mut commit_string));
        commit_string = commit_string.replace("\n", "").replace("\r", "");
        Ok(Commit::from(commit_string))
    }
    fn move_staging_to_master(
        &self,
        repo: &Repo,
        merge_commit: &Commit,
    ) -> Result<(), GitError> {
        if !repo.push_to_master {
            return Ok(());
        }
        try!(self.setup_dir(repo));
        try_cmd!(Command::new(&self.executable), cmd,
        cmd.current_dir(&repo.path)
            .arg("push")
            .arg("-f")
            .arg("origin")
            .arg(format!("{}:{}", merge_commit, &repo.master_branch)));
        Ok(())
    }
    fn setup_dir(&self, repo: &Repo) -> Result<(), GitError> {
        if !Path::new(&repo.path).exists() {
            try_cmd!(Command::new(&self.executable), cmd,
            cmd.arg("init")
                .arg(&repo.path));
            try_cmd!(Command::new(&self.executable), cmd,
            cmd.current_dir(&repo.path)
                .arg("remote")
                .arg("add")
                .arg("origin")
                .arg(&repo.origin));
            try_cmd!(Command::new(&self.executable), cmd,
            cmd.current_dir(&repo.path)
                .arg("config")
                .arg("--local")
                .arg("user.name")
                .arg(&self.name));
            try_cmd!(Command::new(&self.executable), cmd,
            cmd.current_dir(&repo.path)
                .arg("config")
                .arg("--local")
                .arg("user.email")
                .arg(&self.email));
        } else {
            let mut cmd = Command::new(&self.executable);
            cmd.current_dir(&repo.path)
               .arg("merge")
               .arg("--abort");
            info!("Run command: {:?}", cmd);
            try!(cmd.output());
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

pub trait ToShortString {
    fn to_short_string(&self) -> String;
}

impl ToShortString for Commit {
    fn to_short_string(&self) -> String {
        let mut string = self.to_string();
        string.truncate(5);
        string
    }
}
