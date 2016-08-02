// This file is released under the same terms as Rust itself.

use ci;
use crossbeam;
use hyper::client::{Client, IntoUrl};
use hyper::header::{Authorization, Basic};
use pipeline::{self, PipelineId};
use serde_json::from_reader as json_from_reader;
use std::net::TcpListener;
use std::sync::mpsc::{Sender, Receiver};
use util::rate_limited_client::RateLimiter;
use vcs::Commit;

pub trait PipelinesConfig: Send + Sync + 'static {
    fn job_by_pipeline(&self, PipelineId) -> Option<Job>;
    fn pipelines_by_job_name(&self, &str) -> Vec<PipelineId>;
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct Job {
    pub name: String,
    pub token: String,
}

pub struct Worker {
    listen: String,
    host: String,
    pipelines: Box<PipelinesConfig>,
    auth: Option<(String, String)>,
    client: Client,
    rate_limiter: RateLimiter,
}

impl Worker {
    pub fn new(
        listen: String,
        host: String,
        auth: Option<(String, String)>,
        pipelines: Box<PipelinesConfig>,
    ) -> Self {
        Worker {
            listen: listen,
            host: host,
            pipelines: pipelines,
            auth: auth,
            client: Client::default(),
            rate_limiter: RateLimiter::new(),
        }
    }
}

impl<C> pipeline::Worker<ci::Event<C>, ci::Message<C>> for Worker
    where C: 'static + Commit + Sync
{
    fn run(
        &self,
        recv_msg: Receiver<ci::Message<C>>,
        mut send_event: Sender<ci::Event<C>>
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
    fn run_listen<C: 'static + Commit + Sync>(
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
            let pipelines = self.pipelines.pipelines_by_job_name(&desc.name);
            if pipelines.is_empty() {
                warn!("Got result of unknown job: {}", desc.name);
            }
            for pipeline_id in pipelines {
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
                if desc.build.phase == "STARTED" {
                    send_event.send(
                        ci::Event::BuildStarted(
                            pipeline_id,
                            commit,
                            desc.build.full_url.into_url().ok(),
                        )
                    ).expect("Pipeline");
                } else if let Some(ref status) = desc.build.status {
                    match &status[..] {
                        "SUCCESS" => {
                            send_event.send(
                                ci::Event::BuildSucceeded(
                                    pipeline_id,
                                    commit,
                                    desc.build.full_url.into_url().ok(),
                                )
                            ).expect("Pipeline");
                        }
                        e => {
                            info!("Build failed: {}", e);
                            send_event.send(
                                ci::Event::BuildFailed(
                                    pipeline_id,
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

    fn handle_message<C: 'static + Commit + Sync>(
        &self,
        msg: ci::Message<C>,
        send_event: &mut Sender<ci::Event<C>>,
    ) {
        match msg {
            ci::Message::StartBuild(pipeline_id, commit) => {
                let job = match self.pipelines.job_by_pipeline(pipeline_id) {
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
                let result = self.rate_limiter.retry_send(|| {
                    let mut rb = self.client.get(&url);
                    if let Some(ref auth) = self.auth {
                        rb = rb.header(Authorization(Basic{
                            username: auth.0.clone(),
                            password: Some(auth.1.clone()),
                        }));
                    }
                    rb
                });
                match result {
                    Ok(ref res) if !res.status.is_success() => {
                        warn!("Build refused: {:?}", res.status);
                        send_event.send(ci::Event::BuildFailed(
                            pipeline_id,
                            commit,
                            None,
                        )).expect("Pipeline");
                    }
                    Err(e) => {
                        warn!("Failed to contact CI: {:?}", e);
                        send_event.send(ci::Event::BuildFailed(
                            pipeline_id,
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
