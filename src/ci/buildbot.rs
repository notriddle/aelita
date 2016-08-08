// This file is released under the same terms as Rust itself.

use ci;
use crossbeam;
use hyper;
use hyper::buffer::BufReader;
use hyper::client::{Client, IntoUrl, RequestBuilder};
use hyper::header::{Authorization, Basic, Headers, UserAgent};
use hyper::net::{HttpListener, NetworkListener, NetworkStream};
use hyper::server::{Request, Response};
use hyper::status::StatusCode;
use pipeline::{self, PipelineId};
use serde_json;
use serde_json::from_reader as json_from_reader;
use std::fmt::Debug;
use std::io::BufWriter;
use std::sync::mpsc::{Sender, Receiver};
use url::form_urlencoded;
use util::USER_AGENT;
use util::rate_limited_client::RateLimiter;
use vcs::Commit;

/// Buildbot does not actually have a concept of a job;
/// what it has are builders which are triggered by a change hook
/// (we use the poller).
/// We receive notifications from the `HttpStatusPush` plugin, and
/// use the JSON API to determine if all the builders are done.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct Job {
    pub poller: Option<String>,
    pub builders: Vec<String>,
}

pub trait PipelinesConfig: Send + Sync + 'static {
    fn pipelines_by_builder(&self, &str) -> Vec<PipelineId>;
    fn job_by_pipeline(&self, PipelineId) -> Option<Job>;
    fn all(&self) -> Vec<(PipelineId, Job)>;
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
        Worker{
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
    where C: 'static + Commit + Sync, C::Err: Debug
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


#[derive(Deserialize, Serialize)]
struct SourceStampDesc {
    revision: String,
}
#[derive(Deserialize, Serialize)]
struct BuildDesc {
    #[serde(rename="sourceStamps")]
    source_stamps: Vec<SourceStampDesc>,
    text: Option<Vec<String>>,
}

impl Worker {
    fn run_webhook<C: 'static + Commit + Sync>(
        &self,
        send_event: Sender<ci::Event<C>>,
    )
        where C::Err: Debug
    {
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
                    warn!("Got bad webhook: {:?}", e);
                    continue;
                }
            };
            let mut head = Headers::new();
            let res = Response::new(&mut buf_write, &mut head);
            self.handle_webhook(req, res, &send_event);
        }
    }

    fn handle_webhook<C: 'static + Commit + Sync>(
        &self,
        mut _req: Request,
        mut res: Response,
        send_event: &Sender<ci::Event<C>>
    )
        where C::Err: Debug
    {
        info!("Got build status report");
        *res.status_mut() = StatusCode::Ok;
        res.headers_mut()
            .set_raw("Content-Type", vec![b"text/plain".to_vec()]);
        let result = res.send(&[]);
        if let Err(e) = result {
            warn!("Failed to send response: {:?}", e);
        }
        for (pipeline_id, job) in self.pipelines.all() {
            // Check all builder for success.
            let mut is_success = true;
            let mut revision = None;
            for name in &job.builders {
                let result = self.get_current_build(name);
                match result {
                    Ok(builder) => {
                        if let Some(text) = builder.text {
                            if text.len() < 2 {
                                warn!(
                                    "Build {} incomplete, but has text",
                                    name
                                );
                                return;
                            }
                            info!("Builder {} complete", name);
                            is_success = is_success &&
                                text.contains(&"successful".to_string());
                            if let Some(ref revision) = revision {
                                let b = &builder.source_stamps[0].revision[..];
                                if revision != b {
                                    warn!(
                                        "builders different revs: {} and {}",
                                        revision,
                                        &builder.source_stamps[0].revision[..]
                                    );
                                    return;
                                }
                            } else {
                                revision = Some(
                                    builder
                                    .source_stamps[0]
                                    .revision
                                    .clone()
                                );
                            }
                        } else {
                            info!("Builder {} incomplete", name);
                            return;
                        }
                    }
                    Err(e) => {
                        warn!("Builder {} failed check {:?}!", name, e);
                        return;
                    }
                }
            }
            info!("All builders complete: is_success={}", is_success);
            let commit = match C::from_str(&revision.unwrap()[..]) {
                Ok(commit) => commit,
                Err(e) => {
                    warn!("Failed to parse revision: {:?}", e);
                    return;
                }
            };
            // If so, send result to the user
            if is_success {
                send_event.send(
                    ci::Event::BuildSucceeded(
                        pipeline_id,
                        commit,
                        None,
                    )
                ).expect("Pipeline");
            } else {
                send_event.send(
                    ci::Event::BuildFailed(
                        pipeline_id,
                        commit,
                        None,
                    )
                ).expect("Pipeline");
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
                    "{}/change_hook/poller",
                    self.host
                );
                info!("Trigger build: {}", url);
                let body = job.poller.as_ref().map(|poller| {
                    form_urlencoded::Serializer::new(String::new())
                        .append_pair("poller", poller)
                        .finish()
                });
                let result = self.rate_limiter.retry_send(|| {
                    let mut req = self.post(&url);
                    if let &Some(ref body) = &body {
                        req = req.body(body.as_bytes())
                    }
                    req
                });
                match result {
                    Ok(ref result) if !result.status.is_success() => {
                        warn!("Build refused: {:?}", result.status);
                        send_event.send(ci::Event::BuildFailed(
                            pipeline_id,
                            commit,
                            None,
                        )).expect("Pipeline (build refused)");
                    }
                    Err(e) => {
                        warn!("Failed to contact CI: {:?}", e);
                        send_event.send(ci::Event::BuildFailed(
                            pipeline_id,
                            commit,
                            None,
                        )).expect("Pipeline (contact builder failed)");
                    }
                    Ok(_) => {
                        info!("Successfully triggered build");
                        send_event.send(ci::Event::BuildStarted(
                            pipeline_id,
                            commit,
                            None,
                        )).expect("Pipeline (build started)");
                    }
                }
            }
        }
    }

    fn get_current_build(
        &self,
        builder_name: &str,
    ) -> Result<BuildDesc, BuildbotRequestError> {
        let url = format!(
            "{}/json/builders/{}/builds/-1",
            self.host,
            builder_name
        );
        let result = try!(self.rate_limiter.retry_send(|| self.get(&url)));
        if !result.status.is_success() {
            return Err(BuildbotRequestError::HttpStatus(result.status));
        }
        Ok(try!(json_from_reader::<_, BuildDesc>(result)))
    }

    fn post<U: IntoUrl>(&self, url: U) -> RequestBuilder {
        let mut rb = self.client.post(url)
            .header(UserAgent(USER_AGENT.to_owned()));
        if let Some(ref auth) = self.auth {
            rb = rb.header(Authorization(Basic{
                username: auth.0.clone(),
                password: Some(auth.1.clone()),
            }));
        }
        rb
    }

    fn get<U: IntoUrl>(&self, url: U) -> RequestBuilder {
        let mut rb = self.client.get(url)
            .header(UserAgent(USER_AGENT.to_owned()));
        if let Some(ref auth) = self.auth {
            rb = rb.header(Authorization(Basic{
                username: auth.0.clone(),
                password: Some(auth.1.clone()),
            }));
        }
        rb
    }
}

quick_error! {
    #[derive(Debug)]
    pub enum BuildbotRequestError {
        /// HTTP-level error
        HttpStatus(status: StatusCode) {}
        /// HTTP-level error
        Http(err: hyper::error::Error) {
            cause(err)
            from()
        }
        /// JSON error
        Json(err: serde_json::error::Error) {
            cause(err)
            from()
        }
    }
}