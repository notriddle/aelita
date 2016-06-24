// This file is released under the same terms as Rust itself.

//! An implementation of the Common Sense Rule of Software Engineering

#![feature(mpsc_select)]
#![feature(custom_derive, plugin)]
#![plugin(serde_macros)]

extern crate crossbeam;
extern crate env_logger;
#[macro_use] extern crate horrorshow;
extern crate hyper;
#[macro_use] extern crate lazy_static;
#[macro_use] extern crate log;
#[macro_use] extern crate quick_error;
extern crate regex;
extern crate rusqlite;
extern crate serde;
extern crate serde_json;
extern crate toml;
extern crate url;
extern crate void;

mod ci;
mod db;
mod pipeline;
mod ui;
mod util;
mod view;
mod vcs;

use ci::buildbot;
use ci::github_status;
use ci::jenkins;
use pipeline::{Event, GetPipelineId, Pipeline, PipelineId, WorkerThread};
use std::borrow::Cow;
use std::collections::HashMap;
use std::env::args;
use std::error::Error;
use std::fs::File;
use std::io::Read;
use std::path::PathBuf;
use std::process::exit;
use std::str::FromStr;
use std::thread;
use ui::github;
use ui::Pr;
use vcs::git;

macro_rules! try_opt {
    ($e:expr) => (
        match $e {
            Some(e) => e,
            None => return None,
        }
    )
}

macro_rules! expect_opt {
    ($e:expr, $s:expr) => (
        match $e {
            Some(e) => e,
            None => {
                println!($s);
                exit(3);
            }
        }
    )
}

fn main() {
    env_logger::init().unwrap();
    let mut args = args();
    let _ = args.next(); // ignore executable name
    let config_path = args.next()
        .map(Cow::Owned)
        .unwrap_or(Cow::Borrowed("config.toml"));
    let mut config_file = match File::open(&*config_path) {
        Ok(config_file) => config_file,
        Err(e) => {
            println!("Failed to open {}: {}", &*config_path, e);
            exit(1);
        }
    };
    let mut config_string = String::new();
    match config_file.read_to_string(&mut config_string) {
        Ok(_) => {},
        Err(e) => {
            println!("Failed to read {}: {}", &*config_path, e);
            exit(1);
        }
    }
    let config_main = match toml::Parser::new(&config_string).parse() {
        Some(config) => config,
        None => {
            println!("Failed to parse {}", &*config_path);
            exit(2);
        }
    };
    let config = expect_opt!(
        config_main.get("config").and_then(|c| c.as_table()),
        "Invalid configuration file: add a [config] section"
    );
    let config_projects = expect_opt!(
        config_main.get("projects").and_then(|c| c.as_table()),
        "Invalid configuration file: add a [projects] section"
    );
    if config.contains_key("github") {
        run_workers::<GithubCompatibleSetup>(&config, &config_projects);
    } else {
        println!("Please set up one of: github");
        exit(3);
    }
}

fn run_workers<S>(config: &toml::Table, config_projects: &toml::Table) -> !
    where S: CompatibleSetup,
          <<<S as CompatibleSetup>::P as Pr>::C as FromStr>::Err: Error,
          <<S as CompatibleSetup>::P as FromStr>::Err: Error
{
    use std::sync::mpsc::{Select, Handle};
    let db_path = config.get("db")
        .map(|file| file.as_string())
        .unwrap_or_else(|| "db.sqlite".to_owned());
    let mut db = db::sqlite::SqliteDb::open(&db_path).expect("to open up db");
    let workers = S::setup_workers(config, config_projects);
    let (mut pipelines, cis, uis, vcss) =
        workers.start(config, config_projects);
    debug!(
        "Created {} pipelines, {} CIs, {} UIs, and {} VCSs",
        pipelines.len(),
        cis.len(),
        uis.len(),
        vcss.len(),
    );
    start_view::<S::P, _>(
        config,
        config_projects,
        PathBuf::from(db_path)
    );
    unsafe {
        let select = Select::new();
        let mut ci_handles: Vec<Handle<ci::Event<<S::P as Pr>::C>>> =
            cis.iter().map(|worker| {
                select.handle(&worker.recv_event)
            }).collect();
        let mut ui_handles: Vec<Handle<ui::Event<S::P>>> =
            uis.iter().map(|worker| {
                select.handle(&worker.recv_event)
            }).collect();
        let mut vcs_handles: Vec<Handle<vcs::Event<<S::P as Pr>::C>>> =
            vcss.iter().map(|worker| {
                select.handle(&worker.recv_event)
            }).collect();
        // We cannot call add while collecting because the handle is moved.
        for h in &mut ci_handles { h.add(); }
        for h in &mut ui_handles { h.add(); }
        for h in &mut vcs_handles { h.add(); }
        let mut pending: Option<Event<S::P>> = None;
        'outer: loop {
            if let Some(event) = pending.take() {
                let pipeline_id = event.pipeline_id();
                pipelines[pipeline_id.0 as usize].handle_event(&mut db, event);
            }
            let id = select.wait();
            for h in &mut ci_handles {
                if h.id() == id {
                    pending = h.recv().map(Event::CiEvent).ok();
                    continue 'outer;
                }
            }
            for h in &mut ui_handles { 
                if h.id() == id {
                    pending = h.recv().map(Event::UiEvent).ok();
                    continue 'outer;
                }
            }
            for h in &mut vcs_handles { 
                if h.id() == id {
                    pending = h.recv().map(Event::VcsEvent).ok();
                    continue 'outer;
                }
            }
        }
    }
}

fn start_view<P: Pr, Q: Into<PathBuf>>(
    config: &toml::Table,
    config_projects: &toml::Table,
    db_path: Q,
)
    where <P::C as FromStr>::Err: Error,
          <P as FromStr>::Err: Error,
{
    if let Some(view) = config.get("view") {
        let view = expect_opt!(
            view.as_table(),
            "[config.view] must be a table"
        );
        let listen = view.get("listen").map(|listen| {
            expect_opt!(
                listen.as_str(),
                "[config.view.listen] must be a string"
            )
        }).unwrap_or("localhost:80").to_owned();
        let mut pipelines = HashMap::new();
        let mut i = 0;
        for (project_name, project_config) in config_projects.iter() {
            let project_config = project_config.as_table().unwrap();
            pipelines.insert(project_name.to_owned(), PipelineId(i as i32));
            i += 1;
            if project_config.get("try").is_some() {
                pipelines.insert(project_name.to_owned() + "--try", PipelineId(i as i32));
                i += 1;
            }
        }
        let db_path = db_path.into();
        thread::spawn(|| view::run_sqlite::<P>(listen, db_path, pipelines));
    }
}

struct GithubCompatibleSetup {
    github: Option<WorkerThread<
        ui::Event<github::Pr>,
        ui::Message<github::Pr>,
    >>,
    jenkins: Option<WorkerThread<
        ci::Event<git::Commit>,
        ci::Message<git::Commit>,
    >>,
    buildbot: Option<WorkerThread<
        ci::Event<git::Commit>,
        ci::Message<git::Commit>,
    >>,
    github_status: Option<WorkerThread<
        ci::Event<git::Commit>,
        ci::Message<git::Commit>,
    >>,
    git: Option<WorkerThread<
        vcs::Event<git::Commit>,
        vcs::Message<git::Commit>,
    >>,
}

impl GithubCompatibleSetup {

    fn setup_github(
        config: &toml::Table,
        projects: &toml::Table,
    ) -> Option<github::Worker> {
        let github_config = try_opt!(config.get("github").map(|github_config| {
            expect_opt!(
                github_config.as_table(),
                "Invalid [config.github] section: must be a table"
            )
        }));
        let mut github = github::Worker::new(
            expect_opt!(
                github_config.get("listen"),
                "Invalid [config.github] section: no webhook listen address"
            ).as_string(),
            github_config.get("host")
                .map(|x|x.as_string())
                .unwrap_or_else(|| "https://api.github.com".to_owned()),
            expect_opt!(
                github_config.get("token"),
                "Invalid [config.github] section: no authorization token"
            ).as_string(),
            expect_opt!(
                github_config.get("user"),
                "Invalid [config.github] section: no bot username"
            ).as_string(),
        );
        let mut i = 0;
        for (name, def) in projects.iter() {
            let def = expect_opt!(
                def.as_table(),
                "[project] declarations must be tables"
            );
            if let Some(github_def) = def.get("github").map(|github_def| {
                expect_opt!(
                    github_def.as_table(),
                    "[project.github] must be a table"
                )
            }) {
                let owner = github_def.get("owner")
                    .unwrap_or_else(|| {
                        expect_opt!(
                            github_config.get("owner"),
                            "No [config.github.owner] or
                            [project.github.owner]"
                        )
                    })
                    .as_string();
                let repo = github_def.get("repo").map(|r| r.as_string())
                    .unwrap_or_else(|| name.to_owned());
                github.add_project(
                    PipelineId(i as i32),
                    if def.get("try").is_some() {
                        Some(PipelineId(i + 1 as i32))
                    } else {
                        None
                    },
                    github::Repo{
                        owner: owner,
                        repo: repo,
                    },
                );
            }
            if def.get("try").is_some() {
                i += 1;
            }
            i += 1;
        }
        Some(github)
    }

    fn setup_buildbot(
        config: &toml::Table,
        config_projects: &toml::Table,
    ) -> Option<buildbot::Worker> {
        let buildbot_config = try_opt!(
            config.get("buildbot")
                .map(|buildbot_config| {
                    expect_opt!(
                        buildbot_config.as_table(),
                        "Invalid [config.buildbot] section: must be a table"
                    )
                })
        );
        let mut buildbot = buildbot::Worker::new(
            expect_opt!(
                buildbot_config.get("listen"),
                "Invalid [config.buildbot] section: no listen address"
            ).as_string(),
            expect_opt!(
                buildbot_config.get("host"),
                "Invalid [config.buildbot] section: no host address"
            ).as_string(),
            buildbot_config.get("user").map(|user| {
                (user.as_string(), expect_opt!(
                    buildbot_config.get("token"),
                    "Invalid [config.buildbot] section: user, but no password"
                ).as_string())
            }),
        );
        let mut i = 0;
        for (_name, def) in config_projects.iter() {
            let def = expect_opt!(
                def.as_table(),
                "[project] declarations must be tables"
            );
            let buildbot_def = def.get("buildbot").map(|buildbot_def| {
                expect_opt!(
                    buildbot_def.as_table(),
                    "[project.buildbot] must be a table"
                )
            });
            if let Some(buildbot_def) = buildbot_def {
                if let Some(try_def) = def.get("try").map(|def| def.as_table()) {
                    let try_buildbot_def = try_def.and_then(|t| t.get("buildbot"))
                        .map(|b| b.as_table().unwrap()).unwrap();
                    buildbot.add_pipeline(PipelineId(i + 1 as i32), buildbot::Job{
                        poller: try_buildbot_def.get("poller")
                            .map(toml::Value::as_string),
                        builders: expect_opt!(
                            try_buildbot_def.get("builders")
                                .and_then(toml::Value::as_slice)
                                .into_iter().filter(|x| !x.is_empty()).next(),
                            "Invalid [project.try.buildbot]: no builders specified"
                        ).iter().map(toml::Value::as_string).collect(),
                    });
                }
                buildbot.add_pipeline(PipelineId(i as i32), buildbot::Job{
                    poller: buildbot_def.get("poller")
                        .map(toml::Value::as_string),
                    builders: expect_opt!(
                        buildbot_def.get("builders")
                            .and_then(toml::Value::as_slice)
                            .into_iter().filter(|x| !x.is_empty()).next(),
                        "Invalid [project.buildbot]: no builders specified"
                    ).iter().map(toml::Value::as_string).collect(),
                });
            }
            if def.get("try").is_some() {
                i += 1;
            }
            i += 1;
        }
        Some(buildbot)
    }

    fn setup_github_status(
        config: &toml::Table,
        projects: &toml::Table,
    ) -> Option<github_status::Worker> {
        let github_config = try_opt!(config.get("github").map(|github_config| {
            expect_opt!(
                github_config.as_table(),
                "Invalid [config.github] section: must be a table"
            )
        }));
        let github_status_config = match github_config.get("status") {
            Some(github_status_config) => expect_opt!(
                github_status_config.as_table(),
                "Invalid [config.github.status] section: not a table"
            ),
            None => return None,
        };
        let mut github_status = github_status::Worker::new(
            expect_opt!(
                github_status_config.get("listen"),
                "Invalid [config.github.status] section: no listen address"
            ).as_string(),
        );
        let mut i = 0;
        for (name, def) in projects.iter() {
            let def = expect_opt!(
                def.as_table(),
                "[project] declarations must be tables"
            );
            if let Some(github_def) = def.get("github").map(|github_def| {
                expect_opt!(
                    github_def.as_table(),
                    "[project.github] must be a table"
                )
            }) {
                if let Some(context) = github_def.get("status") {
                    let context = context.as_string();
                    let owner = github_def.get("owner")
                        .unwrap_or_else(|| {
                            expect_opt!(
                                github_config.get("owner"),
                                "No [config.github.owner] or
                                [project.github.owner]"
                            )
                        })
                        .as_string();
                    let repo = github_def.get("repo").map(|r| r.as_string())
                        .unwrap_or_else(|| name.to_owned());
                    if def.get("try").is_some() {
                        github_status.add_pipeline(
                            PipelineId(i + 1 as i32),
                            github_status::Repo{
                                repo: repo.clone(),
                                owner: owner.clone(),
                            },
                            context.clone(),
                        );
                    }
                    github_status.add_pipeline(
                        PipelineId(i as i32),
                        github_status::Repo{
                            repo: repo,
                            owner: owner,
                        },
                        context,
                    );
                }
            }
            if def.get("try").is_some() {
                i += 1;
            }
            i += 1;
        }
        Some(github_status)
    }

    fn setup_jenkins(
        config: &toml::Table,
        config_projects: &toml::Table,
    ) -> Option<jenkins::Worker> {
        let jenkins_config = try_opt!(
            config.get("jenkins")
                .map(|jenkins_config| {
                    expect_opt!(
                        jenkins_config.as_table(),
                        "Invalid [config.jenkins] section: must be a table"
                    )
                })
        );
        let mut jenkins = jenkins::Worker::new(
            expect_opt!(
                jenkins_config.get("listen"),
                "Invalid [config.jenkins] section: no listen address"
            ).as_string(),
            expect_opt!(
                jenkins_config.get("host"),
                "Invalid [config.jenkins] section: no host address"
            ).as_string(),
            jenkins_config.get("user").map(|user| {
                (user.as_string(), expect_opt!(
                    jenkins_config.get("token"),
                    "Invalid [config.jenkins] section: user, but no password"
                ).as_string())
            }),
        );
        let mut i = 0;
        for (name, def) in config_projects.iter() {
            let def = expect_opt!(
                def.as_table(),
                "[project] declarations must be tables"
            );
            if let Some(jenkins_def) = def.get("jenkins").map(|jenkins_def| {
                expect_opt!(
                    jenkins_def.as_table(),
                    "[project.jenkins] must be a table"
                )
            }) {
                let name = jenkins_def.get("job").map(|r| r.as_string())
                    .unwrap_or_else(|| name.to_owned());
                let token = expect_opt!(
                    jenkins_def.get("token"),
                    "Invalid [project.jenkins]: no token specified"
                ).as_string();
                if let Some(try_def) = def.get("try").map(|def| def.as_table()) {
                    let try_jenkins_def = try_def.and_then(|t| t.get("jenkins"))
                        .map(|j| j.as_table().unwrap());
                    jenkins.add_pipeline(PipelineId(i + 1 as i32), jenkins::Job{
                        name: try_jenkins_def.and_then(|j| j.get("job"))
                            .map(|r| r.as_string())
                            .unwrap_or_else(|| name.clone() + "--try"),
                        token: try_jenkins_def.and_then(|j| j.get("token"))
                            .map(|r| r.as_string())
                            .unwrap_or_else(|| token.clone()),
                    });
                }
                jenkins.add_pipeline(PipelineId(i as i32), jenkins::Job{
                    name: name,
                    token: token,
                });
            }
            if def.get("try").is_some() {
                i += 1;
            }
            i += 1;
        }
        Some(jenkins)
    }

    fn setup_git(
        config: &toml::Table,
        config_projects: &toml::Table,
    ) -> Option<git::Worker> {
        let git_config = config.get("git").map(|git_config| {
            expect_opt!(
                git_config.as_table(),
                "Invalid [config.git] section: must be a table"
            )
        });
        let github_config = config.get("github").map(|github_config| {
            expect_opt!(
                github_config.as_table(),
                "Invalid [config.github] section: must be a table"
            )
        });
        let mut git = git::Worker::new(
            git_config.and_then(|git_config| git_config.get("executable"))
                .map(|e| e.as_string())
                .unwrap_or_else(|| "git".to_owned()),
            git_config
                .and_then(|git_config| git_config.get("name"))
                .or_else(|| {
                    github_config.and_then(|gc| gc.get("user"))
                })
                .expect("Invalid [config.git] section: no name")
                .as_string(),
            git_config
                .and_then(|git_config| git_config.get("email"))
                .map(|o| o.as_string())
                .or_else(|| {
                    github_config.and_then(|gc| {
                        Some(format!(
                            "{}@github.com",
                            try_opt!(gc.get("user")).as_string()
                        ))
                    })
                }).expect("Invalid [config.git] section: no email"),
        );
        let base_path =
            git_config.and_then(|git_config| git_config.get("path"))
            .map(|p| p.as_string())
            .unwrap_or_else(|| "./cache/".to_owned());
        let mut i = 0;
        for (name, def) in config_projects.iter() {
            let def = expect_opt!(
                def.as_table(),
                "[project] declarations must be tables"
            );
            let git_def = def.get("git").map(|git_def| {
                expect_opt!(
                    git_def.as_table(),
                    "[project.git] must be a table"
                )
            });
            let github_def = def.get("github").map(|github_def| {
                expect_opt!(
                    github_def.as_table(),
                    "[project.github] must be a table"
                )
            });
            if git_def.is_none() && github_def.is_none() {
                continue;
            }
            let path = PathBuf::from(&base_path).join(
                git_def.and_then(
                    |git_def| git_def.get("path")
                ).map(|p| p.as_string())
                .unwrap_or_else(|| name.to_owned())
            );
            let origin = git_def
                .and_then(|git_def| git_def.get("origin"))
                .map(|o| o.as_string())
                .unwrap_or_else(|| {
                    let gdo = github_def.and_then(
                        |github_def| github_def.get("owner")
                    );
                    let gco = github_config.and_then(
                        |github_config| github_config.get("owner")
                    );
                    let owner = gdo.unwrap_or_else(|| expect_opt!(
                        gco,
                        "Invalid [project.git] section: no origin"
                    )).as_string();
                    let gdr = github_def.and_then(
                        |github_def| github_def.get("repo")
                    );
                    let gcr = github_config.and_then(
                        |github_config| github_config.get("repo")
                    );
                    let repo = gdr.and(gcr)
                        .map(|r| r.as_string())
                        .unwrap_or_else(|| name.to_owned());
                    format!("git@github.com:{}/{}.git", owner, repo)
                });
            let master_branch = git_def
                .and_then(|git_def| {
                    git_def.get("master_branch").map(|m| m.as_string())
                })
                .unwrap_or_else(|| "master".to_owned());
            let staging_branch = git_def
                .and_then(|git_def| {
                    git_def.get("staging_branch").map(|m| m.as_string())
                })
                .unwrap_or_else(|| "staging".to_owned());
            if let Some(try_def) = def.get("try").map(|def| def.as_table()) {
                let try_git_def = try_def.and_then(|t| t.get("get"))
                    .map(|b| b.as_table().unwrap());
                let try_path = PathBuf::from(&base_path).join(
                    try_git_def.and_then(
                        |git_def| git_def.get("path")
                    ).map(|p| p.as_string())
                    .unwrap_or_else(|| name.to_owned() + "--try")
                );
                let try_branch = try_git_def
                    .and_then(|git_def| {
                        git_def.get("branch").map(|m| m.as_string())
                    })
                    .unwrap_or_else(|| "trying".to_owned());
                git.add_pipeline(PipelineId(i + 1 as i32), git::Repo{
                    path: try_path,
                    origin: origin.clone(),
                    master_branch: master_branch.clone(),
                    staging_branch: try_branch,
                    push_to_master: false,
                });
            }
            git.add_pipeline(PipelineId(i as i32), git::Repo{
                path: path,
                origin: origin,
                master_branch: master_branch,
                staging_branch: staging_branch,
                push_to_master: true,
            });
            if def.get("try").is_some() {
                i += 1;
            }
            i += 1;
        }
        Some(git)
    }

}

impl CompatibleSetup for GithubCompatibleSetup {
    type P = github::Pr;
    fn setup_workers(config: &toml::Table, projects: &toml::Table) -> Self {
        GithubCompatibleSetup{
            github: GithubCompatibleSetup::setup_github(config, projects)
                .map(|w| WorkerThread::start(w)),
            buildbot: GithubCompatibleSetup::setup_buildbot(config, projects)
                .map(|w| WorkerThread::start(w)),
            github_status:
                GithubCompatibleSetup::setup_github_status(config, projects)
                    .map(|w| WorkerThread::start(w)),
            jenkins: GithubCompatibleSetup::setup_jenkins(config, projects)
                .map(|w| WorkerThread::start(w)),
            git: GithubCompatibleSetup::setup_git(config, projects)
                .map(|w| WorkerThread::start(w)),
        }
    }
    fn start<'a>(&'a self, _config: &toml::Table, projects: &toml::Table)
        -> (
            Vec<Pipeline<
                'a,
                Self::P,
                WorkerThread<
                    ci::Event<<Self::P as Pr>::C>,
                    ci::Message<<Self::P as Pr>::C>,
                >,
                WorkerThread<ui::Event<Self::P>, ui::Message<Self::P>>,
                WorkerThread<
                    vcs::Event<<Self::P as Pr>::C>,
                    vcs::Message<<Self::P as Pr>::C>,
                >,
            >>,
            Vec<&'a WorkerThread<
                ci::Event<<Self::P as Pr>::C>,
                ci::Message<<Self::P as Pr>::C>,
            >>,
            Vec<&'a WorkerThread<ui::Event<Self::P>, ui::Message<Self::P>>>,
            Vec<&'a WorkerThread<
                vcs::Event<<Self::P as Pr>::C>,
                vcs::Message<<Self::P as Pr>::C>,
            >>,
        )
    {
        let mut uis = vec![];
        if let Some(ref g) = self.github { uis.push(g); }
        let mut cis = vec![];
        if let Some(ref b) = self.buildbot { cis.push(b); }
        if let Some(ref g) = self.github_status { cis.push(g); }
        if let Some(ref j) = self.jenkins { cis.push(j); }
        let mut vcss = vec![];
        if let Some(ref g) = self.git { vcss.push(g); }
        let mut pipelines = vec![];
        let mut i = 0;
        for (_name, def) in projects.iter() {
            let def = expect_opt!(
                def.as_table(),
                "[project] declarations must be tables"
            );
            let ui = if def.contains_key("github") {
                expect_opt!(
                    self.github.as_ref(),
                    "[project.github] requires [config.github]"
                )
            } else {
                println!("Project requires at least one UI configured");
                exit(3);
            };
            let ci = if
            (
                def.contains_key("jenkins") &&
                def.contains_key("buildbot")
            ) ||
            (
                def.contains_key("github_status") &&
                def.contains_key("buildbot")
            ) ||
            (
                def.contains_key("jenkins") &&
                def.contains_key("github_status")
            )
            {
                // TODO: A "meta-CI" that can broadcast and aggregate n>1 CIs.
                println!("Multi-CI is not currently supported.");
                exit(3);
            } else if def.contains_key("buildbot") {
                expect_opt!(
                    self.buildbot.as_ref(),
                    "[project.buildbot] requires [config.buildbot]"
                )
            } else if def.get("github")
                    .and_then(|g| g.as_table().unwrap().get("status"))
                    .is_some() {
                expect_opt!(
                    self.github_status.as_ref(),
                    "[project.github_status] requires [config.github_status]"
                )
            } else if def.contains_key("jenkins") {
                expect_opt!(
                    self.jenkins.as_ref(),
                    "[project.jenkins] requires [config.jenkins]"
                )
            } else {
                println!("Project requires at least one CI configured");
                exit(3);
            };
            let vcs = self.git.as_ref().expect("No git setup configured?");
            pipelines.push(Pipeline::new(
                PipelineId(i as i32),
                ci,
                ui,
                vcs,
            ));
            i += 1;
            if def.get("try").is_some() {
                pipelines.push(Pipeline::new(
                    PipelineId(i as i32),
                    ci,
                    ui,
                    vcs,
                ));
                i += 1;
            }
        }
        (pipelines, cis, uis, vcss)
    }
}

trait CompatibleSetup {
    type P: Pr + 'static;
    fn setup_workers(config: &toml::Table, projects: &toml::Table) -> Self;
    fn start<'a>(&'a self, config: &toml::Table, projects: &toml::Table)
        -> (
            Vec<Pipeline<
                'a,
                Self::P,
                WorkerThread<
                    ci::Event<<Self::P as Pr>::C>,
                    ci::Message<<Self::P as Pr>::C>,
                >,
                WorkerThread<ui::Event<Self::P>, ui::Message<Self::P>>,
                WorkerThread<
                    vcs::Event<<Self::P as Pr>::C>,
                    vcs::Message<<Self::P as Pr>::C>,
                >,
            >>,
            Vec<&'a WorkerThread<
                ci::Event<<Self::P as Pr>::C>,
                ci::Message<<Self::P as Pr>::C>,
            >>,
            Vec<&'a WorkerThread<ui::Event<Self::P>, ui::Message<Self::P>>>,
            Vec<&'a WorkerThread<
                vcs::Event<<Self::P as Pr>::C>,
                vcs::Message<<Self::P as Pr>::C>,
            >>,
        );
}

trait AsString {
    fn as_string(&self) -> String;
}

impl AsString for toml::Value {
    fn as_string(&self) -> String {
        match self.as_str() {
            Some(x) => x,
            None => {
                let t = self.type_str();
                println!("Parse error: expected string, found {}", t);
                exit(3);
            }
        }.to_owned()
    }
}