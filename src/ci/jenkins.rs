// This file is released under the same terms as Rust itself.

use ci::{self, CiId};
use crossbeam;
use rest::{authorization, Authorization, Client, IntoUrl};
use pipeline;
use serde_json::from_reader as json_from_reader;
use std::net::TcpListener;
use std::sync::mpsc::{Sender, Receiver};
use util::USER_AGENT;

pub trait PipelinesConfig: Send + Sync + 'static {
    fn job_by_id(&self, CiId) -> Option<Job>;
    fn ids_by_job_name(&self, &str) -> Vec<CiId>;
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct Job {
    pub name: String,
    pub token: String,
}

pub struct Worker {
    listen: String,
    pipelines: Box<PipelinesConfig>,
    client: Client<Authorization<authorization::Basic>>,
}

impl Worker {
    pub fn new(
        listen: String,
        host: String,
        auth: Option<(String, String)>,
        pipelines: Box<PipelinesConfig>,
    ) -> Self {
        let auth = if let Some(auth) = auth {
            (Some(auth.0), Some(auth.1))
        } else {
            (None, None)
        };
        Worker {
            listen: listen,
            pipelines: pipelines,
            client: Client::new(USER_AGENT.to_owned())
                .base(&host)
                .authorization(Authorization(authorization::Basic{
                    username: auth.0.unwrap_or(String::new()),
                    password: auth.1,
                })),
        }
    }
}

impl pipeline::Worker<ci::Event, ci::Message> for Worker {
    fn run(
        &self,
        recv_msg: Receiver<ci::Message>,
        mut send_event: Sender<ci::Event>
    ) {
        crossbeam::scope(|scope| {
            let s2 = &*self;
            let send_event_2 = send_event.clone();
            scope.spawn(move || {
                s2.run_listen(send_event_2);
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
    fn run_listen(
        &self,
        send_event: Sender<ci::Event>,
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
                phase: String,
                status: Option<String>,
                scm: ResultBuildScmDesc,
                full_url: String,
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
            if desc.build.phase != "COMPLETED" &&
               desc.build.phase != "STARTED" {
                info!("Build not completed or started");
                continue;
            }
            let ids = self.pipelines.ids_by_job_name(&desc.name);
            if ids.is_empty() {
                warn!("Got result of unknown job: {}", desc.name);
            }
            for id in ids {
                let commit = desc.build.scm.commit.clone().into();
                if desc.build.phase == "STARTED" {
                    send_event.send(
                        ci::Event::BuildStarted(
                            id,
                            commit,
                            desc.build.full_url.into_url().ok(),
                        )
                    ).expect("Pipeline");
                } else if let Some(ref status) = desc.build.status {
                    match &status[..] {
                        "SUCCESS" => {
                            send_event.send(
                                ci::Event::BuildSucceeded(
                                    id,
                                    commit,
                                    desc.build.full_url.into_url().ok(),
                                )
                            ).expect("Pipeline");
                        }
                        e => {
                            info!("Build failed: {}", e);
                            send_event.send(
                                ci::Event::BuildFailed(
                                    id,
                                    commit,
                                    desc.build.full_url.into_url().ok(),
                                )
                            ).expect("Pipeline");
                        }
                    }
                }
            }
        }
    }

    fn handle_message(
        &self,
        msg: ci::Message,
        send_event: &mut Sender<ci::Event>,
    ) {
        match msg {
            ci::Message::StartBuild(id, commit) => {
                let job = match self.pipelines.job_by_id(id) {
                    Some(job) => job,
                    None => {
                        warn!(
                            "Got start build for bad CI instance {:?}",
                            id
                        );
                        return;
                    },
                };
                let url = format!(
                    "/job/{}/build?token={}",
                    job.name,
                    job.token,
                );
                info!("Trigger build: {}", url);
                let result = self.client
                    .get(&url).expect("valid url")
                    .send();
                match result {
                    Ok(ref res) if !res.is_success() => {
                        warn!("Build refused: {:?}", res.http.status);
                        send_event.send(ci::Event::BuildFailed(
                            id,
                            commit,
                            None,
                        )).expect("Pipeline");
                    }
                    Err(e) => {
                        warn!("Failed to contact CI: {:?}", e);
                        send_event.send(ci::Event::BuildFailed(
                            id,
                            commit,
                            None,
                        )).expect("Pipeline");
                    }
                    Ok(_) => {}
                };
            }
        }
    }

}
