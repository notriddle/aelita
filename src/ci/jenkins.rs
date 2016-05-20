// This file is released under the same terms as Rust itself.

use ci;
use hyper::client::Client;
use hyper::header::{Authorization, Basic};
use pipeline::{self, PipelineId};
use serde_json::from_reader as json_from_reader;
use std::collections::HashMap;
use std::marker::PhantomData;
use std::net::TcpListener;
use std::sync::mpsc::{Sender, Receiver};
use std::thread;
use vcs::Commit;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct Job {
    pub name: String,
    pub token: String,
}

pub struct Worker<C: Commit + 'static> {
    listen: String,
    host: String,
    jobs: HashMap<PipelineId, Job>,
    auth: Option<(String, String)>,
    client: Client,
    _commit: PhantomData<C>,
}

impl<C: Commit + 'static> Worker<C> {
    pub fn new(
        listen: String,
        host: String,
        auth: Option<(String, String)>,
    ) -> Self {
        Worker {
            listen: listen,
            host: host,
            jobs: HashMap::new(),
            auth: auth,
            client: Client::default(),
            _commit: PhantomData,
        }
    }
    pub fn add_pipeline(&mut self, pipeline_id: PipelineId, job: Job) {
        self.jobs.insert(pipeline_id, job);
    }
}

impl<C: Commit + 'static> Clone for Worker<C> {
    fn clone(&self) -> Self {
        Worker {
            listen: self.listen.clone(),
            host: self.host.clone(),
            jobs: self.jobs.clone(),
            auth: self.auth.clone(),
            client: Client::default(),
            _commit: PhantomData,
        }
    }
}

impl<C: Commit + 'static> pipeline::Worker<ci::Event<C>, ci::Message<C>> for Worker<C> {
    fn run(
        &mut self,
        recv_msg: Receiver<ci::Message<C>>,
        mut send_event: Sender<ci::Event<C>>
    ) {
        let send_event_2 = send_event.clone();
        let self_2 = self.clone();
        thread::spawn(move || {
            self_2.run_listen(send_event_2);
        });
        loop {
            self.handle_message(
                recv_msg.recv().expect("Pipeline went away"),
                &mut send_event,
            );
        }
    }
}


impl<C: Commit + 'static> Worker<C> {
    fn run_listen(
        &self,
        send_event: Sender<ci::Event<C>>,
    ) {
        let listener = TcpListener::bind(&self.listen[..]).expect("TCP");
        let mut incoming = listener.incoming();
        while let Some(Ok(stream)) = incoming.next() {
            info!("Got build status notice");
            #[derive(Deserialize, Serialize)]
            struct ResultBuildScmDesc {
                commit: String,
            }
            #[derive(Deserialize, Serialize)]
            struct ResultBuildDesc {
                name: String,
                phase: String,
                status: String,
                scm: ResultBuildScmDesc,
            }
            #[derive(Deserialize, Serialize)]
            struct ResultDesc {
                name: String,
                build: ResultBuildDesc,
            }
            let desc: ResultDesc = match json_from_reader(stream) {
                Ok(desc) => desc,
                Err(e) => {
                    warn!("Result parse failed: {:?}", e);
                    continue;
                }
            };
            if desc.build.phase != "COMPLETED" {
                info!("Build not completed");
                continue;
            }
            let commit = match C::from_str(&desc.build.scm.commit) {
                Ok(commit) => commit,
                Err(_) => {
                    warn!(
                        "Result commit parse failed: {}",
                        desc.build.scm.commit
                    );
                    continue;
                }
            };
            let pipeline_id = {
                let mut pipeline_id = None;
                for (p, job) in &self.jobs {
                    if &job.name == &desc.build.name {
                        pipeline_id = Some(p);
                        break;
                    }
                }
                if let Some(pipeline_id) = pipeline_id {
                    pipeline_id
                } else {
                    warn!(
                        "Got request for nonexistant job: {}",
                        desc.build.name
                    );
                    continue;
                }
            };
            match &desc.build.status[..] {
                "SUCCESS" => {
                    send_event.send(
                        ci::Event::BuildSucceeded(
                            *pipeline_id,
                            commit
                        )
                    ).expect("Pipeline");
                }
                e => {
                    info!("Build failed: {}", e);
                    send_event.send(
                        ci::Event::BuildFailed(
                            *pipeline_id,
                            commit
                        )
                    ).expect("Pipeline");
                }
            }
        }
    }

    fn handle_message(
        &self,
        msg: ci::Message<C>,
        send_event: &mut Sender<ci::Event<C>>,
    ) {
        match msg {
            ci::Message::StartBuild(pipeline_id, commit) => {
                let job = match self.jobs.get(&pipeline_id) {
                    Some(job) => job,
                    None => {
                        warn!(
                            "Got start build for bad pipeline {:?}",
                            pipeline_id
                        );
                        return;
                    },
                };
                let url = format!(
                    "{}/job/{}/build?token={}",
                    self.host,
                    job.name,
                    job.token,
                );
                info!("Trigger build: {}", url);
                let mut rb = self.client.get(&url);
                if let Some(ref auth) = self.auth {
                    rb = rb.header(Authorization(Basic{
                        username: auth.0.clone(),
                        password: Some(auth.1.clone()),
                    }));
                }
                match rb.send() {
                    Ok(ref res) if !res.status.is_success() => {
                        warn!("Build refused: {:?}", res.status);
                        send_event.send(ci::Event::BuildFailed(
                            pipeline_id,
                            commit
                        )).expect("Pipeline");
                    }
                    Err(e) => {
                        warn!("Failed to contact CI: {:?}", e);
                        send_event.send(ci::Event::BuildFailed(
                            pipeline_id,
                            commit
                        )).expect("Pipeline");
                    }
                    Ok(_) => {}
                };
            }
        }
    }

}
