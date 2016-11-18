// This file is released under the same terms as Rust itself.

use ci::{self, CiId, github_status, jenkins};
use config::{PipelineConfig, PipelinesConfig, WorkerBuilder};
use db::{self, DbBox};
use pipeline::{PipelineId, WorkerManager};
use pipeline::WorkerThread;
use std::any::Any;
use std::borrow::Cow;
use std::collections::HashMap;
use std::error::Error;
use std::fmt::{self, Debug, Display};
use std::fs::File;
use std::io::Read;
use std::path::Path;
use toml;
use ui::{self, github};
use vcs::{self, git};
use vcs::github as github_git;
use view;

pub struct GithubBuilder {
    cis: Vec<WorkerThread<
        ci::Event,
        ci::Message,
    >>,
    uis: Vec<WorkerThread<
        ui::Event,
        ui::Message,
    >>,
    vcss: Vec<WorkerThread<
        vcs::Event,
        vcs::Message,
    >>,
    view: Option<WorkerThread<
        view::Event,
        view::Message,
    >>,
    db: DbBox,
    pipelines: StaticPipelinesConfig,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum CiType {
    Jenkins,
    GithubStatus,
}

impl GithubBuilder {
    pub fn build_from_file<P: AsRef<Path>>(path: P)
            -> Result<Self, GithubBuilderError> {
        let path = path.as_ref();
        let mut config_file = match File::open(&*path) {
            Ok(config_file) => config_file,
            Err(e) => return Err(GithubBuilderError::OpenFile(e)),
        };
        let mut config_string = String::new();
        match config_file.read_to_string(&mut config_string) {
            Ok(_) => {},
            Err(e) => return Err(GithubBuilderError::ReadFile(e)),
        }
        let config_main = match toml::Parser::new(&config_string).parse() {
            Some(config) => config,
            None => return Err(GithubBuilderError::Parse),
        };
        Self::build_from_toml(config_main)
    }
    pub fn build_from_toml(config_main: toml::Table)
            -> Result<Self, GithubBuilderError> {
        let config = 
            match config_main.get("config") {
                Some(config) => config,
                None => return Err(GithubBuilderError::NoConfig),
            };
        let config_projects = 
            match config_main.get("projects").and_then(|c| c.as_table()) {
                Some(config_projects) => config_projects,
                None => return Err(GithubBuilderError::NoProjects),
            };
        if config.lookup("github").is_none() {
            return Err(GithubBuilderError::NoConfigGithub);
        }
        let mut github_projects =
            StaticGithubProjectsConfig::new();
        let mut github_status_pipelines =
            StaticGithubStatusPipelinesConfig::new();
        let mut jenkins_pipelines =
            StaticJenkinsPipelinesConfig::new();
        let mut git_pipelines =
            StaticGitPipelinesConfig::new();
        let mut github_git_pipelines =
            StaticGithubGitPipelinesConfig::new();
        let mut view_pipelines =
            StaticViewPipelinesConfig::new();
        let mut pipeline_id = PipelineId(0);
        let mut ci_id = CiId(0);
        let mut ci_to_pipeline: HashMap<CiId, (CiType, PipelineId)> = HashMap::new();
        for (name, def) in config_projects.iter() {
            if def.as_table().is_none() {
                return Err(GithubBuilderError::Project(
                    SetupError::InvalidArg(ProjectArg::Project, Ty::Table)
                ));
            }
            match github_projects.add_project(
                name,
                config,
                def,
                pipeline_id
            ) {
                Ok(()) | Err(SetupError::NotFoundConfig) => {},
                Err(e) => return Err(GithubBuilderError::GithubProject(e)),
            }
            match github_status_pipelines.add_pipeline(
                name,
                config,
                def,
                pipeline_id,
                &mut ci_id,
                &mut ci_to_pipeline,
            ) {
                Ok(()) | Err(SetupError::NotFoundConfig) => {},
                Err(e) =>
                    return Err(GithubBuilderError::GithubStatusProject(e)),
            }
            match jenkins_pipelines.add_pipeline(
                name,
                config,
                def,
                pipeline_id,
                &mut ci_id,
                &mut ci_to_pipeline,
            ) {
                Ok(()) | Err(SetupError::NotFoundConfig) => {},
                Err(e) => return Err(GithubBuilderError::JenkinsProject(e)),
            }
            match git_pipelines.add_pipeline(
                name,
                config,
                def,
                pipeline_id,
                false,
            ) {
                Ok(()) | Err(SetupError::NotFoundConfig) => {},
                Err(e) => return Err(GithubBuilderError::GitProject(e)),
            }
            match github_git_pipelines.add_pipeline(
                name,
                config,
                def,
                pipeline_id,
                false
            ) {
                Ok(()) | Err(SetupError::NotFoundConfig) => {},
                Err(e) => return Err(GithubBuilderError::GithubGitProject(e)),
            }
            match view_pipelines.add_pipeline(
                name,
                config,
                def,
                pipeline_id,
            ) {
                Ok(()) | Err(SetupError::NotFoundConfig) => {},
                Err(e) => return Err(GithubBuilderError::ViewProject(e)),
            }
            pipeline_id.0 += 1;
            if def.lookup("try").is_some() {
                match github_status_pipelines.add_pipeline(
                    name,
                    config,
                    def,
                    pipeline_id,
                    &mut ci_id,
                    &mut ci_to_pipeline,
                ) {
                    Ok(()) | Err(SetupError::NotFoundConfig) => {},
                    Err(e) =>
                        return Err(GithubBuilderError::GithubStatusProject(e)),
                }
                match jenkins_pipelines.add_pipeline(
                    name,
                    config,
                    def,
                    pipeline_id,
                    &mut ci_id,
                    &mut ci_to_pipeline,
                ) {
                    Ok(()) | Err(SetupError::NotFoundConfig) => {},
                    Err(e) =>
                        return Err(GithubBuilderError::JenkinsProject(e)),
                }
                match git_pipelines.add_pipeline(
                    name,
                    config,
                    def,
                    pipeline_id,
                    true
                ) {
                    Ok(()) | Err(SetupError::NotFoundConfig) => {},
                    Err(e) =>
                        return Err(GithubBuilderError::GitProject(e)),
                }
                match github_git_pipelines.add_pipeline(
                    name,
                    config,
                    def,
                    pipeline_id,
                    true
                ) {
                    Ok(()) | Err(SetupError::NotFoundConfig) => {},
                    Err(e) =>
                        return Err(GithubBuilderError::GithubGitProject(e)),
                }
                match view_pipelines.add_pipeline(
                    name,
                    config,
                    def,
                    pipeline_id,
                ) {
                    Ok(()) | Err(SetupError::NotFoundConfig) => {},
                    Err(e) =>
                        return Err(GithubBuilderError::ViewProject(e)),
                }
                pipeline_id.0 += 1;
            }
        }
        let github = match setup_github(config, github_projects) {
            Ok(github) => Some(WorkerThread::start(github)),
            Err(SetupError::NotFoundConfig) => None,
            Err(e) => return Err(GithubBuilderError::Github(e)),
        };
        let github_status =
            match setup_github_status(config, github_status_pipelines) {
                Ok(github_status) => Some(WorkerThread::start(github_status)),
                Err(SetupError::NotFoundConfig) => None,
                Err(e) => return Err(GithubBuilderError::GithubStatus(e)),
            };
        let jenkins = match setup_jenkins(config, jenkins_pipelines) {
            Ok(jenkins) => Some(WorkerThread::start(jenkins)),
            Err(SetupError::NotFoundConfig) => None,
            Err(e) => return Err(GithubBuilderError::Jenkins(e)),
        };
        let git = match setup_git(config, git_pipelines) {
            Ok(git) => Some(WorkerThread::start(git)),
            Err(SetupError::NotFoundConfig) => None,
            Err(e) => return Err(GithubBuilderError::Git(e)),
        };
        let github_git = match setup_github_git(config, github_git_pipelines) {
            Ok(github_git) => Some(WorkerThread::start(github_git)),
            Err(SetupError::NotFoundConfig) => None,
            Err(e) => return Err(GithubBuilderError::GithubGit(e)),
        };
        let view = match setup_view(config, view_pipelines) {
            Ok(view) => Some(WorkerThread::start(view)),
            Err(SetupError::NotFoundConfig) => None,
            Err(e) => return Err(GithubBuilderError::View(e)),
        };
        let mut uis = vec![];
        let github_idx = if let Some(github) = github {
            uis.push(github);
            Some(uis.len()-1)
        } else {
            None
        };
        let mut cis = vec![];
        let github_status_idx = if let Some(github_status) = github_status {
            cis.push(github_status);
            Some(cis.len()-1)
        } else {
            None
        };
        let jenkins_idx = if let Some(jenkins) = jenkins {
            cis.push(jenkins);
            Some(cis.len()-1)
        } else {
            None
        };
        let mut vcss = vec![];
        let git_idx = if let Some(git) = git {
            vcss.push(git);
            Some(vcss.len()-1)
        } else {
            None
        };
        let github_git_idx = if let Some(github_git) = github_git {
            vcss.push(github_git);
            Some(vcss.len()-1)
        } else {
            None
        };
        let mut pipelines = StaticPipelinesConfig::new();
        for (_name, def) in config_projects.iter() {
            let mut pipeline_id = PipelineId(pipelines.0.len() as i32);
            let mut ci_idxs = Vec::new();
            for (&ci_id, &(ci_type, ci_pipeline_id)) in &ci_to_pipeline {
                if ci_pipeline_id == pipeline_id {
                    let ci_idx = match ci_type {
                        CiType::Jenkins => jenkins_idx,
                        CiType::GithubStatus => github_status_idx,
                    };
                    let ci_idx = if let Some(ci_idx) = ci_idx {
                        ci_idx
                    } else {
                        return Err(GithubBuilderError::Dangling);
                    };
                    ci_idxs.push((ci_id, ci_idx));
                }
            }
            if ci_idxs.len() == 0 {
                return Err(GithubBuilderError::Dangling);
            }
            let ui_idx = if def.lookup("github").is_some() {
                if let Some(github_idx) = github_idx {
                    github_idx
                } else {
                    return Err(GithubBuilderError::Dangling);
                }
            } else {
                return Err(GithubBuilderError::Dangling);
            };
            let vcs_idx = if def.lookup("git").is_some() {
                if let Some(git_idx) = git_idx {
                    git_idx
                } else {
                    return Err(GithubBuilderError::Dangling);
                }
            } else if def.lookup("github").is_some() {
                if let Some(github_git_idx) = github_git_idx {
                    github_git_idx
                } else {
                    return Err(GithubBuilderError::Dangling);
                }
            } else {
                return Err(GithubBuilderError::Dangling);
            };
            if def.lookup("try").is_some() {
                pipelines.0.push(PipelineConfig{
                    pipeline_id: pipeline_id,
                    ci: ci_idxs.clone(),
                    ui: ui_idx,
                    vcs: vcs_idx,
                });
            }
            pipelines.0.push(PipelineConfig{
                pipeline_id: pipeline_id,
                ci: ci_idxs,
                ui: ui_idx,
                vcs: vcs_idx,
            });
            pipeline_id.0 = pipeline_id.0 + 1;
        }
        let db_path = config.lookup("db")
            .and_then(|file| file.as_str())
            .unwrap_or_else(|| "db.sqlite");
        let db_build = db::Builder::from_str(db_path)
            .expect("to parse db path");
        let db = db_build
            .open()
            .expect("to open up db");
        Ok(GithubBuilder{
            cis: cis,
            uis: uis,
            vcss: vcss,
            view: view,
            db: db,
            pipelines: pipelines,
        })
    }
}

impl WorkerBuilder for GithubBuilder {
    fn start(self) -> (WorkerManager, DbBox) {
        (
            WorkerManager {
                cis: self.cis,
                uis: self.uis,
                vcss: self.vcss,
                view: self.view,
                pipelines: Box::new(self.pipelines),
            },
            self.db,
        )
    }
}

// Convenience functions for semantic parsing.

macro_rules! toml_arg {
    ($config: expr, $section: expr, $attr: expr, $ty: ident, $arg: expr) => {{
        let section = match $config.lookup($section) {
            Some(section) => section,
            None => return Err(SetupError::NotFoundConfig),
        };
        if !section.as_table().is_some() {
            return Err(SetupError::NotTableConfig)
        }
        let attr = match section.lookup($attr) {
            Some(attr) => attr,
            None => return Err(SetupError::NotFoundArg($arg)),
        };
        match *attr {
            toml::Value::$ty(ref attr) => Clone::clone(attr),
            _ => {
                let ty = match *attr {
                    toml::Value::String(_) => Ty::String,
                    toml::Value::Integer(_) => Ty::Integer,
                    toml::Value::Float(_) => Ty::Float,
                    toml::Value::Boolean(_) => Ty::Boolean,
                    toml::Value::Datetime(_) => Ty::Datetime,
                    toml::Value::Array(_) => Ty::Array,
                    toml::Value::Table(_) => Ty::Table,
                };
                return Err(SetupError::InvalidArg($arg, ty))
            }
        }
    }}
}

macro_rules! toml_arg_default {
    (
        $config: expr,
        $section: expr,
        $attr: expr,
        $ty: ident,
        $arg: expr,
        $default: expr
    ) => {{
        let section = match $config.lookup($section) {
            Some(section) => section,
            None => return Err(SetupError::NotFoundConfig),
        };
        if !section.as_table().is_some() {
            return Err(SetupError::NotTableConfig)
        }
        match section.lookup($attr) {
            Some(attr) =>
                match *attr {
                    toml::Value::$ty(ref attr) => Clone::clone(attr),
                    _ => {
                        let ty = match *attr {
                            toml::Value::String(_) => Ty::String,
                            toml::Value::Integer(_) => Ty::Integer,
                            toml::Value::Float(_) => Ty::Float,
                            toml::Value::Boolean(_) => Ty::Boolean,
                            toml::Value::Datetime(_) => Ty::Datetime,
                            toml::Value::Array(_) => Ty::Array,
                            toml::Value::Table(_) => Ty::Table,
                        };
                        return Err(SetupError::InvalidArg($arg, ty))
                    }
                },
            None => Into::into($default),
        }
    }}
}

// Everything under the [config] section.

fn setup_github(config: &toml::Value, projects: StaticGithubProjectsConfig)
        -> Result<github::Worker, SetupError<GithubArg>> {
    Ok(github::Worker::new(
        toml_arg!(config, "github", "listen", String, GithubArg::Listen),
        toml_arg_default!(config, "github", "host", String, GithubArg::Host,
            "https://api.github.com"
        ),
        toml_arg!(config, "github", "token", String, GithubArg::Token),
        toml_arg!(config, "github", "user", String, GithubArg::User),
        toml_arg!(config, "github", "secret", String, GithubArg::Secret),
        Box::new(projects),
        db::Builder::from_str(
            &toml_arg_default!(config, "github", "db", String, GithubArg::Db,
                config.lookup("db").and_then(toml::Value::as_str)
                    .unwrap_or("db.sqlite").to_owned()
            )[..]
        ).expect("the DB to open"),
    ))
}

fn setup_github_status(
    config: &toml::Value,
    pipelines: StaticGithubStatusPipelinesConfig
) -> Result<github_status::Worker, SetupError<GithubStatusArg>> {
    Ok(github_status::Worker::new(
        toml_arg!(
            config,
            "github.status",
            "listen",
            String,
            GithubStatusArg::Listen
        ),
        toml_arg_default!(
            config,
            "github.status",
            "secret",
            String,
            GithubStatusArg::Secret,
            toml_arg!(
                config,
                "github",
                "secret",
                String,
                GithubStatusArg::Secret
            )
        ),
        Box::new(pipelines),
    ))
}

fn setup_jenkins(
    config: &toml::Value,
    pipelines: StaticJenkinsPipelinesConfig
) -> Result<jenkins::Worker, SetupError<JenkinsArg>> {
    let user = if let Some(user) = config.lookup("jenkins.user") {
        if let Some(user) = user.as_str() {
            Some(user.to_owned())
        } else {
            return Err(SetupError::InvalidArg(JenkinsArg::Token, Ty::String));
        }
    } else {
        None
    };
    let token = if let Some(token) = config.lookup("jenkins.token") {
        if let Some(token) = token.as_str() {
            Some(token.to_owned())
        } else {
            return Err(SetupError::InvalidArg(JenkinsArg::Token, Ty::String));
        }
    } else {
        None
    };
    let auth = if let (Some(user), Some(token)) = (user, token) {
        Some((user, token))
    } else {
        None
    };
    Ok(jenkins::Worker::new(
        toml_arg!(config, "jenkins", "listen", String, JenkinsArg::Listen),
        toml_arg!(config, "jenkins", "host", String, JenkinsArg::Host),
        auth,
        Box::new(pipelines),
    ))
}

fn setup_git(
    config: &toml::Value,
    pipelines: StaticGitPipelinesConfig
) -> Result<git::Worker, SetupError<GitArg>> {
    Ok(git::Worker::new(
        toml_arg_default!(config, "git", "executable",
            String, GitArg::Executable,
            "git"
        ),
        toml_arg_default!(config, "git", "name", String, GitArg::Name,
            toml_arg!(config, "github", "user", String, GitArg::Name)
        ),
        toml_arg_default!(config, "git", "email", String, GitArg::Email,
            match config.lookup("github.user").and_then(toml::Value::as_str) {
                Some(s) => format!("{}@github.com", s),
                None => return Err(SetupError::NotFoundArg(GitArg::Email)),
            }
        ),
        Box::new(pipelines),
    ))
}

fn setup_github_git(
    config: &toml::Value,
    pipelines: StaticGithubGitPipelinesConfig
) -> Result<github_git::Worker, SetupError<GithubGitArg>> {
    Ok(github_git::Worker::new(
        toml_arg_default!(
            config,
            "github.git",
            "host",
            String,
            GithubGitArg::Host,
            toml_arg_default!(
                config,
                "github",
                "host",
                String,
                GithubGitArg::Host,
                "https://api.github.com"
            )
        ),
        toml_arg_default!(
            config,
            "github.git",
            "token",
            String,
            GithubGitArg::Token,
            toml_arg!(config, "github", "token", String, GithubGitArg::Token)
        ),
        Box::new(pipelines),
    ))
}

fn setup_view(
    config: &toml::Value,
    pipelines: StaticViewPipelinesConfig
) -> Result<view::Worker, SetupError<ViewArg>> {
    let auth = if let Some(auth) = config.lookup("view.auth") {
        if auth.as_table().is_none() {
            return Err(SetupError::InvalidArg(ViewArg::Auth, Ty::Table));
        }
        match auth.lookup("type").and_then(toml::Value::as_str) {
            Some("github") =>  view::Auth::Github(
                toml_arg!(config, "view.auth", "app_id", String,
                    ViewArg::AuthGithubAppId
                ),
                toml_arg!(config, "view.auth", "app_secret", String,
                    ViewArg::AuthGithubAppSecret
                ),
                toml_arg!(config, "view.auth", "organization", String,
                    ViewArg::AuthGithubOrganization
                ),
            ),
            None => view::Auth::None,
            _ => return Err(SetupError::InvalidArg(
                ViewArg::AuthType,
                Ty::String
            )),
        }
    } else {
        view::Auth::None
    };
    Ok(view::Worker::new(
        toml_arg!(config, "view", "listen", String, ViewArg::Listen),
        db::Builder::from_str(
            config.lookup("db").and_then(toml::Value::as_str)
                .unwrap_or("db.sqlite")
        ).expect("DB to work"),
        Box::new(pipelines),
        toml_arg!(config, "view", "secret", String, ViewArg::Secret),
        auth,
    ))
}

// Everything under the [projects] section.

struct StaticPipelinesConfig(Vec<PipelineConfig>);

impl StaticPipelinesConfig {
    fn new() -> Self {
        StaticPipelinesConfig(Vec::new())
    }
}

impl PipelinesConfig for StaticPipelinesConfig {
    fn by_pipeline_id(&self, id: PipelineId) -> PipelineConfig {
        for cfg in &self.0 {
            if cfg.pipeline_id == id {
                return cfg.clone();
            }
        }
        panic!("Invalid pipeline ID: {:?}", id);
    }
    fn by_ci_id(&self, id: CiId) -> PipelineConfig {
        for cfg in &self.0 {
            for &ci in &cfg.ci {
                if ci.0 == id {
                    return cfg.clone();
                }
            }
        }
        panic!("Invalid ci ID: {:?}", id);
    }
    fn len(&self) -> usize {
        self.0.len()
    }
}

struct StaticGithubProjectsConfig(
    HashMap<github::Repo, github::RepoPipelines>
);

impl StaticGithubProjectsConfig {
    fn new() -> Self {
        StaticGithubProjectsConfig(HashMap::new())
    }
    fn add_project(
        &mut self,
        name: &str,
        config: &toml::Value,
        def: &toml::Value,
        pipeline_id: PipelineId
    ) -> Result<(), SetupError<GithubProjectArg>> {
        self.0.insert(
            github::Repo{
                owner: toml_arg_default!(
                    config,
                    "github",
                    "owner",
                    String,
                    GithubProjectArg::Owner,
                    toml_arg!(
                        def,
                        "github",
                        "owner",
                        String,
                        GithubProjectArg::Owner
                    )
                ),
                repo: toml_arg_default!(
                    def,
                    "github",
                    "repo",
                    String,
                    GithubProjectArg::Repo,
                    name
                )
            },
            github::RepoPipelines{
                pipeline_id: pipeline_id,
                try_pipeline_id: if def.lookup("try").is_some() {
                    Some(PipelineId(pipeline_id.0 + 1))
                } else {
                    None
                },
            }
        );
        Ok(())
    }
}

impl github::ProjectsConfig for StaticGithubProjectsConfig {
    fn pipelines_by_repo(
        &self,
        repo: &github::Repo
    ) -> Option<github::RepoPipelines> {
        self.0.get(repo).map(Clone::clone)
    }
    fn repo_by_pipeline(&self, pipeline_id: PipelineId)
            -> Option<(github::Repo, github::PipelineType)> {
        for (repo, pipelines) in self.0.iter() {
            if pipelines.pipeline_id == pipeline_id {
                return Some((repo.clone(), github::PipelineType::Stage));
            }
            if pipelines.try_pipeline_id == Some(pipeline_id) {
                return Some((repo.clone(), github::PipelineType::Try));
            }
        }
        return None;
    }
}


struct StaticGithubStatusPipelinesConfig(
    HashMap<CiId, github_status::Repo>
);

impl StaticGithubStatusPipelinesConfig {
    fn new() -> Self {
        StaticGithubStatusPipelinesConfig(HashMap::new())
    }
    fn add_pipeline(
        &mut self,
        name: &str,
        config: &toml::Value,
        def: &toml::Value,
        pipeline_id: PipelineId,
        ci_id: &mut CiId,
        ci_to_pipeline: &mut HashMap<CiId, (CiType, PipelineId)>,
    ) -> Result<(), SetupError<GithubStatusProjectArg>> {
        match def.lookup("github.status") {
            Some(gh) => match gh {
                &toml::Value::String(ref context) => {
                    self.add_item(
                        name,
                        config,
                        def,
                        context,
                        pipeline_id,
                        ci_id,
                        ci_to_pipeline,
                    )
                }
                &toml::Value::Array(ref contexts) => {
                    for context in contexts {
                        if let &toml::Value::String(ref context) = context {
                            try!(self.add_item(
                                name,
                                config,
                                def,
                                context,
                                pipeline_id,
                                ci_id,
                                ci_to_pipeline,
                            ))
                        } else {
                            return Err(SetupError::InvalidArg(
                                GithubStatusProjectArg::Context,
                                Ty::String,
                            ));
                        }
                    }
                    Ok(())
                }
                _ => Err(SetupError::NotTableConfig)
            },
            None => Err(SetupError::NotFoundConfig),
        }
    }
    fn add_item(
        &mut self,
        name: &str,
        config: &toml::Value,
        def: &toml::Value,
        context: &str,
        pipeline_id: PipelineId,
        ci_id: &mut CiId,
        ci_to_pipeline: &mut HashMap<CiId, (CiType, PipelineId)>,
    ) -> Result<(), SetupError<GithubStatusProjectArg>> {
        let repo = github_status::Repo{
            owner: toml_arg_default!(
                def,
                "github",
                "owner",
                String,
                GithubStatusProjectArg::Owner,
                toml_arg!(
                    config,
                    "github",
                    "owner",
                    String,
                    GithubStatusProjectArg::Owner
                )
            ),
            repo: toml_arg_default!(
                def,
                "github",
                "repo",
                String,
                GithubStatusProjectArg::Repo,
                name
            ),
            context: context.to_owned(),
        };
        self.0.entry(*ci_id).or_insert(repo);
        ci_to_pipeline.insert(*ci_id, (CiType::GithubStatus, pipeline_id));
        ci_id.0 += 1;
        Ok(())
    }
}

impl github_status::PipelinesConfig for StaticGithubStatusPipelinesConfig {
    fn repo_by_id(&self, id: CiId)
            -> Option<github_status::Repo> {
        return self.0.get(&id).map(Clone::clone)
    }
    fn ids_by_repo(
        &self,
        repo: &github_status::Repo
    ) -> Vec<CiId> {
        let mut ret_val = vec![];
        for (id, i_repo) in self.0.iter() {
            if repo == i_repo {
                ret_val.push(*id)
            }
        }
        ret_val
    }
}


struct StaticJenkinsPipelinesConfig(
    HashMap<CiId, jenkins::Job>
);

impl StaticJenkinsPipelinesConfig {
    fn new() -> Self {
        StaticJenkinsPipelinesConfig(HashMap::new())
    }
    fn add_pipeline(
        &mut self,
        name: &str,
        config: &toml::Value,
        def: &toml::Value,
        pipeline_id: PipelineId,
        ci_id: &mut CiId,
        ci_to_pipeline: &mut HashMap<CiId, (CiType, PipelineId)>,
    ) -> Result<(), SetupError<JenkinsProjectArg>> {
        match def.lookup("jenkins") {
            Some(gh) => match gh {
                jenkins_def @ &toml::Value::Table(_) => {
                    self.add_item(
                        name,
                        config,
                        def,
                        jenkins_def,
                        pipeline_id,
                        ci_id,
                        ci_to_pipeline,
                    )
                }
                &toml::Value::Array(ref jenkins_defs) => {
                    for jenkins_def in jenkins_defs {
                        try!(self.add_item(
                            name,
                            config,
                            def,
                            jenkins_def,
                            pipeline_id,
                            ci_id,
                            ci_to_pipeline,
                        ))
                    }
                    Ok(())
                }
                _ => Err(SetupError::NotTableConfig)
            },
            None => Err(SetupError::NotFoundConfig),
        }
    }
    fn add_item(
        &mut self,
        name: &str,
        _config: &toml::Value,
        _def: &toml::Value,
        jenkins_def: &toml::Value,
        pipeline_id: PipelineId,
        ci_id: &mut CiId,
        ci_to_pipeline: &mut HashMap<CiId, (CiType, PipelineId)>,
    ) -> Result<(), SetupError<JenkinsProjectArg>> {
        let job = jenkins::Job{
            name: toml_arg_default!(
                jenkins_def,
                "",
                "name",
                String,
                JenkinsProjectArg::Name,
                name
            ),
            token: toml_arg!(
                jenkins_def,
                "",
                "token",
                String,
                JenkinsProjectArg::Token
            ),
        };
        self.0.entry(*ci_id).or_insert(job);
        ci_to_pipeline.insert(*ci_id, (CiType::Jenkins, pipeline_id));
        ci_id.0 += 1;
        Ok(())
    }
}

impl jenkins::PipelinesConfig for StaticJenkinsPipelinesConfig {
    fn job_by_id(&self, id: CiId)
            -> Option<jenkins::Job> {
        return self.0.get(&id).map(Clone::clone)
    }
    fn ids_by_job_name(&self, job_name: &str) -> Vec<CiId> {
        let mut ret_val = vec![];
        for (id, i_job) in self.0.iter() {
            if job_name == i_job.name {
                ret_val.push(*id)
            }
        }
        ret_val
    }
}


struct StaticGitPipelinesConfig(
    HashMap<PipelineId, git::Repo>
);

impl StaticGitPipelinesConfig {
    fn new() -> Self {
        StaticGitPipelinesConfig(HashMap::new())
    }
    fn add_pipeline(
        &mut self,
        name: &str,
        config: &toml::Value,
        def: &toml::Value,
        pipeline_id: PipelineId,
        is_try: bool,
    ) -> Result<(), SetupError<GitProjectArg>> {
        let repo = git::Repo{
            path: toml_arg_default!(
                config,
                "git",
                "path",
                String,
                GitProjectArg::Path,
                "cache/"
            ) + &toml_arg_default!(
                def,
                "git",
                "path",
                String,
                GitProjectArg::Path,
                name
            ),
            origin: toml_arg!(
                def,
                "git",
                "origin",
                String,
                GitProjectArg::Origin
            ),
            master_branch: toml_arg_default!(
                def,
                "git",
                "master_branch",
                String,
                GitProjectArg::MasterBranch,
                "master"
            ),
            staging_branch: toml_arg_default!(
                def,
                if is_try { "try.git" } else { "git" },
                if is_try { "branch" } else { "staging_branch" },
                String,
                GitProjectArg::StagingBranch,
                if is_try { "trying" } else { "staging" }
            ),
            push_to_master: !is_try
        };
        self.0.entry(pipeline_id).or_insert(repo);
        Ok(())
    }
}

impl git::PipelinesConfig for StaticGitPipelinesConfig {
    fn repo_by_pipeline(&self, pipeline_id: PipelineId)
            -> Option<git::Repo> {
        return self.0.get(&pipeline_id).map(Clone::clone)
    }
}


struct StaticGithubGitPipelinesConfig(
    HashMap<PipelineId, github_git::Repo>
);

impl StaticGithubGitPipelinesConfig {
    fn new() -> Self {
        StaticGithubGitPipelinesConfig(HashMap::new())
    }
    fn add_pipeline(
        &mut self,
        name: &str,
        config: &toml::Value,
        def: &toml::Value,
        pipeline_id: PipelineId,
        is_try: bool,
    ) -> Result<(), SetupError<GithubGitProjectArg>> {
        let repo = github_git::Repo{
            owner: toml_arg_default!(
                config,
                "github",
                "owner",
                String,
                GithubGitProjectArg::Owner,
                toml_arg!(
                    def,
                    "github",
                    "owner",
                    String,
                    GithubGitProjectArg::Owner
                )
            ),
            repo: toml_arg_default!(
                def,
                "github",
                "repo",
                String,
                GithubGitProjectArg::Repo,
                name
            ),
            master_branch: toml_arg_default!(
                def,
                "github",
                "master_branch",
                String,
                GithubGitProjectArg::MasterBranch,
                "master"
            ),
            staging_branch: toml_arg_default!(
                def,
                if is_try { "try.github" } else { "github" },
                if is_try { "branch" } else { "staging_branch" },
                String,
                GithubGitProjectArg::StagingBranch,
                if is_try { "trying" } else { "staging" }
            ),
            push_to_master: !is_try
        };
        self.0.entry(pipeline_id).or_insert(repo);
        Ok(())
    }
}

impl github_git::PipelinesConfig for StaticGithubGitPipelinesConfig {
    fn repo_by_pipeline(&self, pipeline_id: PipelineId)
            -> Option<github_git::Repo> {
        return self.0.get(&pipeline_id).map(Clone::clone)
    }
}

struct StaticViewPipelinesConfig(HashMap<String, PipelineId>);

impl StaticViewPipelinesConfig {
    fn new() -> Self {
        StaticViewPipelinesConfig(HashMap::new())
    } 
    fn add_pipeline(
        &mut self,
        name: &str,
        _config: &toml::Value,
        _def: &toml::Value,
        pipeline_id: PipelineId,
    ) -> Result<(), SetupError<ViewProjectArg>> {
        self.0.insert(name.to_owned(), pipeline_id);
        Ok(())
    }
}

impl view::PipelinesConfig for StaticViewPipelinesConfig {
    fn pipeline_by_name(&self, name: &str) -> Option<PipelineId> {
        self.0.get(name).map(|x| *x)
    }
    fn all(&self) -> Vec<(Cow<str>, PipelineId)> {
        self.0.iter().map(|x| (Cow::Borrowed(&x.0[..]), *x.1)).collect()
    }
}

// Errors and args definitions.

quick_error! {
    #[derive(Debug)]
    pub enum GithubBuilderError {
        OpenFile(err: ::std::io::Error) {
            cause(err)
        }
        ReadFile(err: ::std::io::Error) {
            cause(err)
        }
        Parse {}
        NoConfig {}
        NoProjects {}
        NoConfigGithub {}
        Dangling {}
        Github(err: SetupError<GithubArg>) {
            cause(err)
        }
        GithubStatus(err: SetupError<GithubStatusArg>) {
            cause(err)
        }
        Jenkins(err: SetupError<JenkinsArg>) {
            cause(err)
        }
        Git(err: SetupError<GitArg>) {
            cause(err)
        }
        GithubGit(err: SetupError<GithubGitArg>) {
            cause(err)
        }
        View(err: SetupError<ViewArg>) {
            cause(err)
        }
        Project(err: SetupError<ProjectArg>) {
            cause(err)
        }
        GithubProject(err: SetupError<GithubProjectArg>) {
            cause(err)
        }
        GithubStatusProject(err: SetupError<GithubStatusProjectArg>) {
            cause(err)
        }
        JenkinsProject(err: SetupError<JenkinsProjectArg>) {
            cause(err)
        }
        GitProject(err: SetupError<GitProjectArg>) {
            cause(err)
        }
        GithubGitProject(err: SetupError<GithubGitProjectArg>) {
            cause(err)
        }
        ViewProject(err: SetupError<ViewProjectArg>) {
            cause(err)
        }
    }
}

#[derive(Debug)]
pub enum Ty {
    String,
    Integer,
    Float,
    Boolean,
    Datetime,
    Array,
    Table,
}

#[derive(Debug)]
pub enum GithubArg {
    Listen,
    Host,
    Token,
    User,
    Secret,
    Db,
}

#[derive(Debug)]
pub enum GithubStatusArg {
    Listen,
    Secret,
}

#[derive(Debug)]
pub enum JenkinsArg {
    Listen,
    Host,
    Token,
}

#[derive(Debug)]
pub enum GitArg {
    Executable,
    Name,
    Email,
}

#[derive(Debug)]
pub enum GithubGitArg {
    Host,
    Token,
}

#[derive(Debug)]
pub enum ViewArg {
    Listen,
    Secret,
    Auth,
    AuthType,
    AuthGithubAppId,
    AuthGithubAppSecret,
    AuthGithubOrganization,
}

#[derive(Debug)]
pub enum ProjectArg {
    Project,
}

#[derive(Debug)]
pub enum GithubProjectArg {
    Owner,
    Repo,
}

#[derive(Debug)]
pub enum GithubStatusProjectArg {
    Owner,
    Repo,
    Context,
}

#[derive(Debug)]
pub enum JenkinsProjectArg {
    Name,
    Token,
}

#[derive(Debug)]
pub enum GitProjectArg {
    Path,
    Origin,
    MasterBranch,
    StagingBranch,
}

#[derive(Debug)]
pub enum GithubGitProjectArg {
    Owner,
    Repo,
    MasterBranch,
    StagingBranch,
}

#[derive(Debug)]
pub enum ViewProjectArg {}

#[derive(Debug)]
pub enum SetupError<T: Debug> {
    NotFoundConfig,
    NotTableConfig,
    NotFoundArg(T),
    InvalidArg(T, Ty),
}

impl<T: Any + Debug> Error for SetupError<T> {
    fn description(&self) -> &str {
        match *self {
            SetupError::NotFoundConfig => "Config not found",
            SetupError::NotTableConfig => "Config is not a table",
            SetupError::NotFoundArg(_) => "Argument not found",
            SetupError::InvalidArg(_, _) => "Argument of wrong type",
        }
    }
}

impl<T: Any + Debug> Display for SetupError<T> {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        Display::fmt(self.description(), fmt)
    }
}