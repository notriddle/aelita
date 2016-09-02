// This file is released under the same terms as Rust itself.

use ci::{self, github_status, jenkins};
use config::{PipelineConfig, WorkerBuilder};
use db::{self, DbBox};
use pipeline::WorkerManager;
use pipeline::WorkerThread;
use std::error::Error;
use ui::{self, github};
use vcs::{self, git};
use vcs::github as github_git;
use view;

pub struct GithubBuilder {
    ci: WorkerThread<
        ci::Event<git::Commit>,
        ci::Message<git::Commit>,
    >,
    ui: WorkerThread<
        ui::Event<github::Pr>,
        ui::Message<github::Pr>,
    >,
    vcs: WorkerThread<
        vcs::Event<git::Commit>,
        vcs::Message<git::Commit>,
    >,
    view: WorkerThread<
        view::Event,
        view::Message,
    >,
    db: DbBox<github::Pr>,
    pipelines: Box<PipelineConfig>,
}

macro_rules! try_env {
    ($env: expr, $key: expr, $keyname: ident) => (
        match $env($key) {
            Some(value) => value,
            None => return Err(GithubBuilderError::MissingKey(
                GithubBuilderKey::$keyname
            )),
        }
    )
}

impl GithubBuilder {
    pub fn build_from_os_env()
            -> Result<Self, GithubBuilderError> {
        GithubBuilder::build_from_env(|var| {
            let var = "AELITA_".to_owned() + var;
            ::std::env::var(&var[..]).ok()
        })
    }
    pub fn build_from_env<F: Fn(&str) -> Option<String>>(env: F)
            -> Result<Self, GithubBuilderError> {
        if try_env!(env, "UI_TYPE", UiType) != "github" {
            return Err(GithubBuilderError::NotGithub);
        }
        let db_key = try_env!(env, "PIPELINE_DB", PipelineDb);
        let db_builder = match db::Builder::from_str(&db_key[..]) {
            Ok(db_builder) => db_builder,
            Err(e) => return Err(GithubBuilderError::DbConnect(e)),
        };
        let db = match db_builder.open() {
            Ok(db) => db,
            Err(e) => return Err(GithubBuilderError::DbConnect(e)),
        };
        let pj_key = try_env!(env, "PROJECT_DB", ProjectDb);
        let pj_builder = match db::Builder::from_str(&pj_key[..]) {
            Ok(pj_builder) => pj_builder,
            Err(e) => return Err(GithubBuilderError::PjConnect(e)),
        };
        let pipelines: Box<PipelineConfig> = match pj_builder {
            db::Builder::Sqlite(d) =>
                Box::new(try!(sqlite::PipelineConfig::new(d))),
            db::Builder::Postgres(d) =>
                Box::new(try!(postgres::PipelineConfig::new(d))),
        };
        Ok(GithubBuilder{
            ci: try!(setup_ci(&env)),
            ui: try!(setup_github(&env)),
            vcs: try!(setup_vcs(&env)),
            view: try!(setup_view(&env)),
            db: db,
            pipelines: pipelines,
        })
    }
}

impl WorkerBuilder for GithubBuilder {
    type Pr = github::Pr;
    fn start(self) -> (WorkerManager<Self::Pr>, DbBox<Self::Pr>) {
        (
            WorkerManager {
                cis: vec![self.ci],
                uis: vec![self.ui],
                vcss: vec![self.vcs],
                view: Some(self.view),
                pipelines: self.pipelines,
            },
            self.db,
        )
    }
}

fn setup_github<F: Fn(&str) -> Option<String>>(env: &F) -> Result<
    WorkerThread<ui::Event<github::Pr>, ui::Message<github::Pr>>,
    GithubBuilderError,
> {
    let pj_key = try_env!(env, "PROJECT_DB", ProjectDb);
    let pj_builder = match db::Builder::from_str(&pj_key[..]) {
        Ok(pj_builder) => pj_builder,
        Err(e) => return Err(GithubBuilderError::PjConnect(e)),
    };
    let projects: Box<github::ProjectsConfig> = match pj_builder {
        db::Builder::Sqlite(d) =>
            Box::new(try!(sqlite::GithubProjectsConfig::new(d))),
        db::Builder::Postgres(d) =>
            Box::new(try!(postgres::GithubProjectsConfig::new(d))),
    };
    let gh_key = try_env!(env, "UI_GITHUB_DB", UiGithubDb);
    let gh_builder = match db::Builder::from_str(&gh_key[..]) {
        Ok(gh_builder) => gh_builder,
        Err(e) => return Err(GithubBuilderError::GhConnect(e)),
    };
    Ok(WorkerThread::start(github::Worker::new(
        try_env!(env, "UI_GITHUB_LISTEN", UiGithubListen),
        try_env!(env, "UI_GITHUB_HOST", UiGithubHost),
        try_env!(env, "UI_GITHUB_TOKEN", UiGithubToken),
        try_env!(env, "UI_GITHUB_USER", UiGithubUser),
        try_env!(env, "UI_GITHUB_SECRET", UiGithubSecret),
        projects,
        gh_builder,
    )))
}

fn setup_ci<F: Fn(&str) -> Option<String>>(env: &F) -> Result<
    WorkerThread<ci::Event<git::Commit>, ci::Message<git::Commit>>,
    GithubBuilderError,
> {
    match &try_env!(env, "CI_TYPE", CiType)[..] {
        "jenkins" => setup_jenkins(env),
        "github_status" => setup_github_status(env),
        _ => Err(GithubBuilderError::InvalidKey(GithubBuilderKey::CiType)),
    }
}

fn setup_jenkins<F: Fn(&str) -> Option<String>>(env: &F) -> Result<
    WorkerThread<ci::Event<git::Commit>, ci::Message<git::Commit>>,
    GithubBuilderError,
> {
    let pj_key = try_env!(env, "PROJECT_DB", ProjectDb);
    let pj_builder = match db::Builder::from_str(&pj_key[..]) {
        Ok(pj_builder) => pj_builder,
        Err(e) => return Err(GithubBuilderError::PjConnect(e)),
    };
    let pipelines: Box<jenkins::PipelinesConfig> = match pj_builder {
        db::Builder::Sqlite(d) =>
            Box::new(try!(sqlite::JenkinsPipelinesConfig::new(d))),
        db::Builder::Postgres(d) =>
            Box::new(try!(postgres::JenkinsPipelinesConfig::new(d))),
    };
    Ok(WorkerThread::start(jenkins::Worker::new(
        try_env!(env, "CI_JENKINS_LISTEN", CiJenkinsListen),
        try_env!(env, "CI_JENKINS_HOST", CiJenkinsHost),
        Some((
            try_env!(env, "CI_JENKINS_USER", CiJenkinsUser),
            try_env!(env, "CI_JENKINS_TOKEN", CiJenkinsToken),
        )),
        pipelines,
    )))
}

fn setup_github_status<F: Fn(&str) -> Option<String>>(env: &F) -> Result<
    WorkerThread<ci::Event<git::Commit>, ci::Message<git::Commit>>,
    GithubBuilderError,
> {
    let pj_key = try_env!(env, "PROJECT_DB", ProjectDb);
    let pj_builder = match db::Builder::from_str(&pj_key[..]) {
        Ok(pj_builder) => pj_builder,
        Err(e) => return Err(GithubBuilderError::PjConnect(e)),
    };
    let pipelines: Box<github_status::PipelinesConfig> = match pj_builder {
        db::Builder::Sqlite(d) =>
            Box::new(try!(sqlite::GithubStatusPipelinesConfig::new(d))),
        db::Builder::Postgres(d) =>
            Box::new(try!(postgres::GithubStatusPipelinesConfig::new(d))),
    };
    Ok(WorkerThread::start(github_status::Worker::new(
        try_env!(env, "CI_GITHUB_LISTEN", CiGithubListen),
        try_env!(env, "CI_GITHUB_SECRET", CiGithubSecret),
        pipelines,
    )))
}

fn setup_vcs<F: Fn(&str) -> Option<String>>(env: &F) -> Result<
    WorkerThread<vcs::Event<git::Commit>, vcs::Message<git::Commit>>,
    GithubBuilderError,
> {
    match &try_env!(env, "VCS_TYPE", VcsType)[..] {
        "git" => setup_git(env),
        "github" => setup_github_git(env),
        _ => Err(GithubBuilderError::InvalidKey(GithubBuilderKey::VcsType)),
    }
}

fn setup_github_git<F: Fn(&str) -> Option<String>>(env: &F) -> Result<
    WorkerThread<vcs::Event<git::Commit>, vcs::Message<git::Commit>>,
    GithubBuilderError,
> {
    let pj_key = try_env!(env, "PROJECT_DB", ProjectDb);
    let pj_builder = match db::Builder::from_str(&pj_key[..]) {
        Ok(pj_builder) => pj_builder,
        Err(e) => return Err(GithubBuilderError::PjConnect(e)),
    };
    let pipelines: Box<github_git::PipelinesConfig> = match pj_builder {
        db::Builder::Sqlite(d) =>
            Box::new(try!(sqlite::GithubGitPipelinesConfig::new(d))),
        db::Builder::Postgres(d) =>
            Box::new(try!(postgres::GithubGitPipelinesConfig::new(d))),
    };
    Ok(WorkerThread::start(github_git::Worker::new(
        try_env!(env, "VCS_GITHUB_HOST", VcsGithubHost),
        try_env!(env, "VCS_GITHUB_TOKEN", VcsGithubToken),
        pipelines,
    )))
}

fn setup_git<F: Fn(&str) -> Option<String>>(env: &F) -> Result<
    WorkerThread<vcs::Event<git::Commit>, vcs::Message<git::Commit>>,
    GithubBuilderError,
> {
    let pj_key = try_env!(env, "PROJECT_DB", ProjectDb);
    let pj_builder = match db::Builder::from_str(&pj_key[..]) {
        Ok(pj_builder) => pj_builder,
        Err(e) => return Err(GithubBuilderError::PjConnect(e)),
    };
    let pipelines: Box<git::PipelinesConfig> = match pj_builder {
        db::Builder::Sqlite(d) =>
            Box::new(try!(sqlite::GitPipelinesConfig::new(d))),
        db::Builder::Postgres(d) =>
            Box::new(try!(postgres::GitPipelinesConfig::new(d))),
    };
    Ok(WorkerThread::start(git::Worker::new(
        try_env!(env, "VCS_GIT_EXECUTABLE", VcsGitExecutable),
        try_env!(env, "VCS_GIT_NAME", VcsGitName),
        try_env!(env, "VCS_GIT_EMAIL", VcsGitEmail),
        pipelines,
    )))
}

fn setup_view<F: Fn(&str) -> Option<String>>(env: &F) -> Result<
    WorkerThread<view::Event, view::Message>,
    GithubBuilderError,
> {
    let db_key = try_env!(env, "PIPELINE_DB", PipelineDb);
    let db_builder = match db::Builder::from_str(&db_key[..]) {
        Ok(db_builder) => db_builder,
        Err(e) => return Err(GithubBuilderError::DbConnect(e)),
    };
    let pj_key = try_env!(env, "PROJECT_DB", ProjectDb);
    let pj_builder = match db::Builder::from_str(&pj_key[..]) {
        Ok(pj_builder) => pj_builder,
        Err(e) => return Err(GithubBuilderError::PjConnect(e)),
    };
    let pipelines: Box<view::PipelinesConfig> = match pj_builder {
        db::Builder::Sqlite(d) =>
            Box::new(try!(sqlite::ViewPipelinesConfig::new(d))),
        db::Builder::Postgres(d) =>
            Box::new(try!(postgres::ViewPipelinesConfig::new(d))),
    };
    Ok(WorkerThread::start(view::Worker::<github::Pr>::new(
        try_env!(env, "VIEW_LISTEN", ViewListen),
        db_builder,
        pipelines,
        try_env!(env, "VIEW_SECRET", ViewSecret),
        view::Auth::None,
    )))
}

mod sqlite {
    use config::PipelineConfig as TPipelineConfig;
    use pipeline::PipelineId;
    use rusqlite::Connection;
    use std::borrow::Cow;
    use std::error::Error;
    use std::path::PathBuf;
    use std::sync::Mutex;
    use ui::github::{self, ProjectsConfig as TGithubProjectsConfig};
    use ci::jenkins::{self, PipelinesConfig as TJenkinsPipelinesConfig};
    use ci::github_status;
    use ci::github_status::PipelinesConfig as TGithubStatusPipelinesConfig;
    use vcs::git;
    use vcs::git::PipelinesConfig as TGitPipelinesConfig;
    use vcs::github as github_git;
    use vcs::github::PipelinesConfig as TGithubGitPipelinesConfig;
    use view::{PipelinesConfig as TViewPipelinesConfig};
    pub struct PipelineConfig {
        conn: Mutex<Connection>,
    }
    impl PipelineConfig {
        pub fn new(path: PathBuf)
                -> Result<Self, Box<Error + Send + Sync + 'static>>
        {
            let conn = try!(Connection::open(&path));
            try!(conn.execute_batch(r###"
                CREATE TABLE IF NOT EXISTS twelvef_config_pipeline (
                    pipeline_id INTEGER PRIMARY KEY,
                    name TEXT,
                    UNIQUE (name)
                );
            "###));
            Ok(PipelineConfig{
                conn: Mutex::new(conn),
            })
        }
    }
    impl TPipelineConfig for PipelineConfig {
        fn workers_by_id(&self, _: PipelineId) -> (usize, usize, usize) {
            (0, 0, 0)
        }
        fn len(&self) -> usize {
            let conn = self.conn.lock().unwrap();
            let sql = r###"
                SELECT COUNT(*)
                FROM twelvef_config_pipeline
            "###;
            let mut stmt = conn.prepare(&sql)
                .expect("Prepare pipeline count query");
            let mut rows = stmt
                .query_map(&[], |row| row.get::<_, i32>(0) as usize)
                .expect("Get pipeline count");
            rows.next().map(|row| row.expect("SQLite to work")).unwrap()
        }
    }
    pub struct GithubProjectsConfig {
        conn: Mutex<Connection>,
    }
    impl GithubProjectsConfig {
        pub fn new(path: PathBuf)
                -> Result<Self, Box<Error + Send + Sync + 'static>>
        {
            let conn = try!(Connection::open(path));
            try!(conn.execute_batch(r###"
                CREATE TABLE IF NOT EXISTS twelvef_github_projects (
                    pipeline_id INTEGER PRIMARY KEY,
                    try_pipeline_id INTEGER NULL,
                    owner TEXT,
                    repo TEXT,
                    UNIQUE (owner, repo)
                );
            "###));
            Ok(GithubProjectsConfig{
                conn: Mutex::new(conn),
            })
        }
    }
    impl TGithubProjectsConfig for GithubProjectsConfig {
        fn pipelines_by_repo(&self, repo: &github::Repo)
                -> Option<github::RepoPipelines>
        {
            let conn = self.conn.lock().unwrap();
            let sql = r###"
                SELECT pipeline_id, try_pipeline_id
                FROM twelvef_github_projects
                WHERE owner = ? AND repo = ?
            "###;
            let mut stmt = conn.prepare(&sql)
                .expect("prepare pipelines query");
            let mut rows = stmt
                .query_map(&[&repo.owner, &repo.repo], |row| {
                    github::RepoPipelines{
                        pipeline_id:
                            PipelineId(row.get::<_, i32>(0)),
                        try_pipeline_id:
                            row.get::<_, Option<i32>>(1).map(PipelineId),
                    }
                })
                .expect("get pipelines");
            rows.next().map(|row| row.expect("sqlite to work"))
        }
        fn repo_by_pipeline(&self, pipeline_id: PipelineId)
                -> Option<(github::Repo, github::PipelineType)>
        {
            let conn = self.conn.lock().unwrap();
            let sql = r###"
                SELECT owner, repo, pipeline_id
                FROM twelvef_github_projects
                WHERE pipeline_id = ? OR try_pipeline_id = ?
            "###;
            let mut stmt = conn.prepare(&sql)
                .expect("prepare repo query");
            let mut rows = stmt
                .query_map(&[&pipeline_id.0, &pipeline_id.0], |row| {
                    let pipeline_type =
                        if row.get::<_, i32>(2) == pipeline_id.0 {
                            github::PipelineType::Stage
                        } else {
                            github::PipelineType::Try
                        };
                    (
                        github::Repo{
                            owner:
                                row.get::<_, String>(0),
                            repo:
                                row.get::<_, String>(1),
                        },
                        pipeline_type
                    )
                })
                .expect("get repo");
            rows.next().map(|row| row.expect("sqlite to work"))
        }
    }
    pub struct JenkinsPipelinesConfig {
        conn: Mutex<Connection>,
    }
    impl JenkinsPipelinesConfig {
        pub fn new(path: PathBuf)
                -> Result<Self, Box<Error + Send + Sync + 'static>>
        {
            let conn = try!(Connection::open(path));
            try!(conn.execute_batch(r###"
                CREATE TABLE IF NOT EXISTS twelvef_jenkins_pipelines (
                    pipeline_id INTEGER PRIMARY KEY,
                    name TEXT,
                    token TEXT
                );
            "###));
            Ok(JenkinsPipelinesConfig{
                conn: Mutex::new(conn),
            })
        }
    }
    impl TJenkinsPipelinesConfig for JenkinsPipelinesConfig {
        fn job_by_pipeline(&self, pipeline_id: PipelineId)
                -> Option<jenkins::Job>
        {
            let conn = self.conn.lock().unwrap();
            let sql = r###"
                SELECT name, token
                FROM twelvef_jenkins_pipelines
                WHERE pipeline_id = ?
            "###;
            let mut stmt = conn.prepare(&sql)
                .expect("prepare job query");
            let mut rows = stmt
                .query_map(&[&pipeline_id.0], |row| {
                    jenkins::Job{
                        name:
                            row.get::<_, String>(0),
                        token:
                            row.get::<_, String>(1),
                    }
                })
                .expect("get job");
            rows.next().map(|row| row.expect("sqlite to work"))
        }
        fn pipelines_by_job_name(&self, job: &str)
                -> Vec<PipelineId>
        {
            let conn = self.conn.lock().unwrap();
            let sql = r###"
                SELECT pipeline_id
                FROM twelvef_jenkins_pipelines
                WHERE name = ?
            "###;
            let mut stmt = conn.prepare(&sql)
                .expect("prepare pipelines query");
            let rows = stmt
                .query_map(&[&job], |row| PipelineId(row.get::<_, i32>(0)))
                .expect("get pipelines");
            let rows = rows.map(|row| row.expect("sqlite to work")).collect();
            rows
        }
    }
    pub struct GithubStatusPipelinesConfig {
        conn: Mutex<Connection>,
    }
    impl GithubStatusPipelinesConfig {
        pub fn new(path: PathBuf)
                -> Result<Self, Box<Error + Send + Sync + 'static>>
        {
            let conn = try!(Connection::open(path));
            try!(conn.execute_batch(r###"
                CREATE TABLE IF NOT EXISTS twelvef_github_status_pipelines (
                    pipeline_id INTEGER PRIMARY KEY,
                    owner TEXT,
                    repo TEXT,
                    context TEXT
                );
            "###));
            Ok(GithubStatusPipelinesConfig{
                conn: Mutex::new(conn),
            })
        }
    }
    impl TGithubStatusPipelinesConfig for GithubStatusPipelinesConfig {
        fn repo_by_pipeline(&self, pipeline_id: PipelineId)
                -> Option<github_status::Repo>
        {
            let conn = self.conn.lock().unwrap();
            let sql = r###"
                SELECT owner, repo, context
                FROM twelvef_github_status_pipelines
                WHERE pipeline_id = ?
            "###;
            let mut stmt = conn.prepare(&sql)
                .expect("prepare repo query");
            let mut rows = stmt
                .query_map(&[&pipeline_id.0], |row| {
                    github_status::Repo{
                        owner:
                            row.get::<_, String>(0),
                        repo:
                            row.get::<_, String>(1),
                        context:
                            row.get::<_, String>(2),
                    }
                })
                .expect("get repo");
            rows.next().map(|row| row.expect("sqlite to work"))
        }
        fn pipelines_by_repo(&self, repo: &github_status::Repo)
                -> Vec<PipelineId>
        {
            let conn = self.conn.lock().unwrap();
            let sql = r###"
                SELECT pipeline_id
                FROM twelvef_github_status_pipelines
                WHERE owner = ? AND repo = ? AND context = ?
            "###;
            let mut stmt = conn.prepare(&sql)
                .expect("prepare pipelines query");
            let rows = stmt
                .query_map(&[&repo.owner, &repo.repo, &repo.context], |row| {
                    PipelineId(row.get::<_, i32>(0))
                })
                .expect("get pipelines");
            let rows = rows.map(|row| row.expect("sqlite to work")).collect();
            rows
        }
    }
    pub struct GithubGitPipelinesConfig {
        conn: Mutex<Connection>,
    }
    impl GithubGitPipelinesConfig {
        pub fn new(path: PathBuf)
                -> Result<Self, Box<Error + Send + Sync + 'static>>
        {
            let conn = try!(Connection::open(path));
            try!(conn.execute_batch(r###"
                CREATE TABLE IF NOT EXISTS twelvef_github_git_pipelines (
                    pipeline_id INTEGER PRIMARY KEY,
                    owner TEXT,
                    repo TEXT,
                    master_branch TEXT,
                    staging_branch TEXT,
                    push_to_master BOOLEAN
                );
            "###));
            Ok(GithubGitPipelinesConfig{
                conn: Mutex::new(conn),
            })
        }
    }
    impl TGithubGitPipelinesConfig for GithubGitPipelinesConfig {
        fn repo_by_pipeline(&self, pipeline_id: PipelineId)
                -> Option<github_git::Repo>
        {
            let conn = self.conn.lock().unwrap();
            let sql = r###"
                SELECT
                    owner,
                    repo,
                    master_branch,
                    staging_branch,
                    push_to_master
                FROM twelvef_github_git_pipelines
                WHERE pipeline_id = ?
            "###;
            let mut stmt = conn.prepare(&sql)
                .expect("prepare repo query");
            let mut rows = stmt
                .query_map(&[&pipeline_id.0], |row| {
                    github_git::Repo{
                        owner:
                            row.get::<_, String>(0),
                        repo:
                            row.get::<_, String>(1),
                        master_branch:
                            row.get::<_, String>(2),
                        staging_branch:
                            row.get::<_, String>(3),
                        push_to_master:
                            row.get::<_, bool>(4),
                    }
                })
                .expect("get repo");
            rows.next().map(|row| row.expect("sqlite to work"))
        }
    }
    pub struct GitPipelinesConfig {
        conn: Mutex<Connection>,
    }
    impl GitPipelinesConfig {
        pub fn new(path: PathBuf)
                -> Result<Self, Box<Error + Send + Sync + 'static>>
        {
            let conn = try!(Connection::open(path));
            try!(conn.execute_batch(r###"
                CREATE TABLE IF NOT EXISTS twelvef_git_pipelines (
                    pipeline_id INTEGER PRIMARY KEY,
                    path TEXT,
                    origin TEXT,
                    master_branch TEXT,
                    staging_branch TEXT,
                    push_to_master TEXT
                );
            "###));
            Ok(GitPipelinesConfig{
                conn: Mutex::new(conn),
            })
        }
    }
    impl TGitPipelinesConfig for GitPipelinesConfig {
        fn repo_by_pipeline(&self, pipeline_id: PipelineId)
                -> Option<git::Repo>
        {
            let conn = self.conn.lock().unwrap();
            let sql = r###"
                SELECT
                    path,
                    origin,
                    master_branch,
                    staging_branch,
                    push_to_master
                FROM twelvef_git_pipelines
                WHERE pipeline_id = ?
            "###;
            let mut stmt = conn.prepare(&sql)
                .expect("prepare repo query");
            let mut rows = stmt
                .query_map(&[&pipeline_id.0], |row| {
                    git::Repo{
                        path:
                            row.get::<_, String>(0),
                        origin:
                            row.get::<_, String>(1),
                        master_branch:
                            row.get::<_, String>(2),
                        staging_branch:
                            row.get::<_, String>(3),
                        push_to_master:
                            row.get::<_, bool>(4),
                    }
                })
                .expect("get repo");
            rows.next().map(|row| row.expect("sqlite to work"))
        }
    }
    pub struct ViewPipelinesConfig {
        conn: Mutex<Connection>,
    }
    impl ViewPipelinesConfig {
        pub fn new(path: PathBuf)
                -> Result<Self, Box<Error + Send + Sync + 'static>>
        {
            let conn = try!(Connection::open(path));
            Ok(ViewPipelinesConfig{
                conn: Mutex::new(conn),
            })
        }
    }
    impl TViewPipelinesConfig for ViewPipelinesConfig {
        fn pipeline_by_name(&self, name: &str)
                -> Option<PipelineId>
        {
            let conn = self.conn.lock().unwrap();
            let sql = r###"
                SELECT pipeline_id
                FROM twelvef_config_pipeline
                WHERE name = ?
            "###;
            let mut stmt = conn.prepare(&sql)
                .expect("prepare repo query");
            let mut rows = stmt
                .query_map(&[&name], |row| {
                    PipelineId(row.get::<_, i32>(0))
                })
                .expect("get repo");
            rows.next().map(|row| row.expect("sqlite to work"))
        }
        fn all(&self) -> Vec<(Cow<str>, PipelineId)> {
            let conn = self.conn.lock().unwrap();
            let sql = r###"
                SELECT name, pipeline_id
                FROM twelvef_config_pipeline
            "###;
            let mut stmt = conn.prepare(&sql)
                .expect("prepare repos query");
            let rows = stmt
                .query_map(&[], |row| (
                    Cow::Owned(row.get::<_, String>(0)),
                    PipelineId(row.get::<_, i32>(1)),
                ))
                .expect("get repos");
            let rows = rows.map(|row| row.expect("sqlite to work")).collect();
            rows
        }
    }
}

mod postgres {
    use config::PipelineConfig as TPipelineConfig;
    use pipeline::PipelineId;
    use postgres::{Connection, ConnectParams, IntoConnectParams, SslMode};
    use std::borrow::Cow;
    use std::error::Error;
    use ui::github::{self, ProjectsConfig as TGithubProjectsConfig};
    use ci::jenkins::{self, PipelinesConfig as TJenkinsPipelinesConfig};
    use ci::github_status;
    use ci::github_status::PipelinesConfig as TGithubStatusPipelinesConfig;
    use vcs::git;
    use vcs::git::PipelinesConfig as TGitPipelinesConfig;
    use vcs::github as github_git;
    use vcs::github::PipelinesConfig as TGithubGitPipelinesConfig;
    use view::{PipelinesConfig as TViewPipelinesConfig};
    pub struct PipelineConfig {
        params: ConnectParams,
    }
    impl PipelineConfig {
        pub fn new<Q: IntoConnectParams>(params: Q)
                -> Result<Self, Box<Error + Send + Sync + 'static>>
        {
            let result = PipelineConfig{
                params: try!(params.into_connect_params()),
            };
            try!(try!(result.conn()).batch_execute(r###"
                CREATE TABLE IF NOT EXISTS twelvef_config_pipeline (
                    pipeline_id SERIAL PRIMARY KEY,
                    name TEXT,
                    UNIQUE (name)
                );
            "###));
            Ok(result)
        }
        fn conn(&self) -> Result<Connection, Box<Error + Send + Sync>> {
            Ok(try!(Connection::connect(self.params.clone(), SslMode::None)))
        }
    }
    impl TPipelineConfig for PipelineConfig {
        fn workers_by_id(&self, _: PipelineId) -> (usize, usize, usize) {
            (0, 0, 0)
        }
        fn len(&self) -> usize {
            retry!{{
                let conn = retry_unwrap!(self.conn());
                let sql = r###"
                    SELECT COUNT(*)
                    FROM twelvef_config_pipeline
                "###;
                let stmt = retry_unwrap!(conn.prepare(&sql));
                let rows = retry_unwrap!(stmt.query(&[]));
                let rows = rows.iter();
                let mut rows = rows.map(|row| {
                    row.get::<_, i64>(0) as usize
                });
                rows.next().unwrap()
            }}
        }
    }
    pub struct GithubProjectsConfig {
        params: ConnectParams,
    }
    impl GithubProjectsConfig {
        pub fn new<Q: IntoConnectParams>(params: Q)
                -> Result<Self, Box<Error + Send + Sync + 'static>>
        {
            let result = GithubProjectsConfig{
                params: try!(params.into_connect_params()),
            };
            try!(try!(result.conn()).batch_execute(r###"
                CREATE TABLE IF NOT EXISTS twelvef_github_projects (
                    pipeline_id INTEGER PRIMARY KEY,
                    try_pipeline_id INTEGER NULL,
                    owner TEXT,
                    repo TEXT,
                    UNIQUE (owner, repo)
                );
            "###));
            Ok(result)
        }
        fn conn(&self) -> Result<Connection, Box<Error + Send + Sync>> {
            Ok(try!(Connection::connect(self.params.clone(), SslMode::None)))
        }
    }
    impl TGithubProjectsConfig for GithubProjectsConfig {
        fn pipelines_by_repo(&self, repo: &github::Repo)
                -> Option<github::RepoPipelines>
        {
            retry!{{
                let conn = retry_unwrap!(self.conn());
                let sql = r###"
                    SELECT pipeline_id, try_pipeline_id
                    FROM twelvef_github_projects
                    WHERE owner = $1 AND repo = $2
                "###;
                let stmt = retry_unwrap!(conn.prepare(&sql));
                let rows = retry_unwrap!(
                    stmt.query(&[&repo.owner, &repo.repo])
                );
                let rows = rows.iter();
                let mut rows = rows.map(|row| {
                    github::RepoPipelines{
                        pipeline_id:
                            PipelineId(row.get::<_, i32>(0)),
                        try_pipeline_id:
                            row.get::<_, Option<i32>>(1).map(PipelineId),
                    }
                });
                rows.next()
            }}
        }
        fn repo_by_pipeline(&self, pipeline_id: PipelineId)
                -> Option<(github::Repo, github::PipelineType)>
        {
            retry!{{
                let conn = retry_unwrap!(self.conn());
                let sql = r###"
                    SELECT owner, repo, pipeline_id
                    FROM twelvef_github_projects
                    WHERE pipeline_id = $1 OR try_pipeline_id = $2
                "###;
                let stmt = retry_unwrap!(conn.prepare(&sql));
                let rows = retry_unwrap!(stmt.query(
                    &[&pipeline_id.0, &pipeline_id.0]
                ));
                let rows = rows.iter();
                let mut rows = rows.map(|row| {
                    let pipeline_type =
                        if row.get::<_, i32>(2) == pipeline_id.0 {
                            github::PipelineType::Stage
                        } else {
                            github::PipelineType::Try
                        };
                    (
                        github::Repo{
                            owner:
                                row.get::<_, String>(0),
                            repo:
                                row.get::<_, String>(1),
                        },
                        pipeline_type
                    )
                });
                rows.next()
            }}
        }
    }
    pub struct JenkinsPipelinesConfig {
        params: ConnectParams,
    }
    impl JenkinsPipelinesConfig {
        pub fn new<Q: IntoConnectParams>(params: Q)
                -> Result<Self, Box<Error + Send + Sync + 'static>>
        {
            let result = JenkinsPipelinesConfig{
                params: try!(params.into_connect_params()),
            };
            try!(try!(result.conn()).batch_execute(r###"
                CREATE TABLE IF NOT EXISTS twelvef_jenkins_pipelines (
                    pipeline_id INTEGER PRIMARY KEY,
                    name TEXT,
                    token TEXT
                );
            "###));
            Ok(result)
        }
        fn conn(&self) -> Result<Connection, Box<Error + Send + Sync>> {
            Ok(try!(Connection::connect(self.params.clone(), SslMode::None)))
        }
    }
    impl TJenkinsPipelinesConfig for JenkinsPipelinesConfig {
        fn job_by_pipeline(&self, pipeline_id: PipelineId)
                -> Option<jenkins::Job>
        {
            retry!{{
                let conn = retry_unwrap!(self.conn());
                let sql = r###"
                    SELECT name, token
                    FROM twelvef_jenkins_pipelines
                    WHERE pipeline_id = $1
                "###;
                let stmt = retry_unwrap!(conn.prepare(&sql));
                let rows = retry_unwrap!(stmt.query(&[&pipeline_id.0]));
                let rows = rows.iter();
                let mut rows = rows.map(|row| {
                    jenkins::Job{
                        name:
                            row.get::<_, String>(0),
                        token:
                            row.get::<_, String>(1),
                    }
                });
                rows.next()
            }}
        }
        fn pipelines_by_job_name(&self, job: &str)
                -> Vec<PipelineId>
        {
            retry!{{
                let conn = retry_unwrap!(self.conn());
                let sql = r###"
                    SELECT pipeline_id
                    FROM twelvef_jenkins_pipelines
                    WHERE name = $1
                "###;
                let stmt = retry_unwrap!(conn.prepare(&sql));
                let rows = retry_unwrap!(stmt.query(&[&job]));
                let rows = rows.iter();
                let rows = rows.map(|row| {
                    PipelineId(row.get::<_, i32>(0))
                });
                let rows = rows.collect();
                rows
            }}
        }
    }
    pub struct GithubStatusPipelinesConfig {
        params: ConnectParams,
    }
    impl GithubStatusPipelinesConfig {
        pub fn new<Q: IntoConnectParams>(params: Q)
                -> Result<Self, Box<Error + Send + Sync + 'static>>
        {
            let result = GithubStatusPipelinesConfig{
                params: try!(params.into_connect_params()),
            };
            try!(try!(result.conn()).batch_execute(r###"
                CREATE TABLE IF NOT EXISTS twelvef_github_status_pipelines (
                    pipeline_id INTEGER PRIMARY KEY,
                    owner TEXT,
                    repo TEXT,
                    context TEXT
                );
            "###));
            Ok(result)
        }
        fn conn(&self) -> Result<Connection, Box<Error + Send + Sync>> {
            Ok(try!(Connection::connect(self.params.clone(), SslMode::None)))
        }
    }
    impl TGithubStatusPipelinesConfig for GithubStatusPipelinesConfig {
        fn repo_by_pipeline(&self, pipeline_id: PipelineId)
                -> Option<github_status::Repo>
        {
            retry!{{
                let conn = retry_unwrap!(self.conn());
                let sql = r###"
                    SELECT owner, repo, context
                    FROM twelvef_github_status_pipelines
                    WHERE pipeline_id = $1
                "###;
                let stmt = retry_unwrap!(conn.prepare(&sql));
                let rows = retry_unwrap!(stmt.query(&[&pipeline_id.0]));
                let rows = rows.iter();
                let mut rows = rows.map(|row| {
                    github_status::Repo{
                        owner:
                            row.get::<_, String>(0),
                        repo:
                            row.get::<_, String>(1),
                        context:
                            row.get::<_, String>(2),
                    }
                });
                rows.next()
            }}
        }
        fn pipelines_by_repo(&self, repo: &github_status::Repo)
                -> Vec<PipelineId>
        {
            retry!{{
                let conn = retry_unwrap!(self.conn());
                let sql = r###"
                    SELECT pipeline_id
                    FROM twelvef_github_status_pipelines
                    WHERE owner = $1 AND repo = $2 AND context = $3
                "###;
                let stmt = retry_unwrap!(conn.prepare(&sql));
                let rows = retry_unwrap!(
                    stmt.query(&[&repo.owner, &repo.repo, &repo.context])
                );
                let rows = rows.iter();
                let rows = rows.map(|row| {
                    PipelineId(row.get::<_, i32>(0))
                });
                let rows = rows.collect();
                rows
            }}
        }
    }
    pub struct GithubGitPipelinesConfig {
        params: ConnectParams,
    }
    impl GithubGitPipelinesConfig {
        pub fn new<Q: IntoConnectParams>(params: Q)
                -> Result<Self, Box<Error + Send + Sync + 'static>>
        {
            let result = GithubGitPipelinesConfig{
                params: try!(params.into_connect_params()),
            };
            try!(try!(result.conn()).batch_execute(r###"
                CREATE TABLE IF NOT EXISTS twelvef_github_git_pipelines (
                    pipeline_id INTEGER PRIMARY KEY,
                    owner TEXT,
                    repo TEXT,
                    master_branch TEXT,
                    staging_branch TEXT,
                    push_to_master BOOLEAN
                );
            "###));
            Ok(result)
        }
        fn conn(&self) -> Result<Connection, Box<Error + Send + Sync>> {
            Ok(try!(Connection::connect(self.params.clone(), SslMode::None)))
        }
    }
    impl TGithubGitPipelinesConfig for GithubGitPipelinesConfig {
        fn repo_by_pipeline(&self, pipeline_id: PipelineId)
                -> Option<github_git::Repo>
        {
            retry!{{
                let conn = retry_unwrap!(self.conn());
                let sql = r###"
                    SELECT
                        owner,
                        repo,
                        master_branch,
                        staging_branch,
                        push_to_master
                    FROM twelvef_github_git_pipelines
                    WHERE pipeline_id = $1
                "###;
                let stmt = retry_unwrap!(conn.prepare(&sql));
                let rows = retry_unwrap!(stmt.query(&[&pipeline_id.0]));
                let rows = rows.iter();
                let mut rows = rows.map(|row| {
                    github_git::Repo{
                        owner:
                            row.get::<_, String>(0),
                        repo:
                            row.get::<_, String>(1),
                        master_branch:
                            row.get::<_, String>(2),
                        staging_branch:
                            row.get::<_, String>(3),
                        push_to_master:
                            row.get::<_, bool>(4),
                    }
                });
                rows.next()
            }}
        }
    }
    pub struct GitPipelinesConfig {
        params: ConnectParams,
    }
    impl GitPipelinesConfig {
        pub fn new<Q: IntoConnectParams>(params: Q)
                -> Result<Self, Box<Error + Send + Sync + 'static>>
        {
            let result = GitPipelinesConfig{
                params: try!(params.into_connect_params()),
            };
            try!(try!(result.conn()).batch_execute(r###"
                CREATE TABLE IF NOT EXISTS twelvef_git_pipelines (
                    pipeline_id INTEGER PRIMARY KEY,
                    path TEXT,
                    origin TEXT,
                    master_branch TEXT,
                    staging_branch TEXT,
                    push_to_master TEXT
                );
            "###));
            Ok(result)
        }
        fn conn(&self) -> Result<Connection, Box<Error + Send + Sync>> {
            Ok(try!(Connection::connect(self.params.clone(), SslMode::None)))
        }
    }
    impl TGitPipelinesConfig for GitPipelinesConfig {
        fn repo_by_pipeline(&self, pipeline_id: PipelineId)
                -> Option<git::Repo>
        {
            retry!{{
                let conn = retry_unwrap!(self.conn());
                let sql = r###"
                    SELECT
                        path,
                        origin,
                        master_branch,
                        staging_branch,
                        push_to_master
                    FROM twelvef_git_pipelines
                    WHERE pipeline_id = $1
                "###;
                let stmt = retry_unwrap!(conn.prepare(&sql));
                let rows = retry_unwrap!(stmt.query(&[&pipeline_id.0]));
                let rows = rows.iter();
                let mut rows = rows.map(|row| {
                    git::Repo{
                        path:
                            row.get::<_, String>(0),
                        origin:
                            row.get::<_, String>(1),
                        master_branch:
                            row.get::<_, String>(2),
                        staging_branch:
                            row.get::<_, String>(3),
                        push_to_master:
                            row.get::<_, bool>(4),
                    }
                });
                rows.next()
            }}
        }
    }
    pub struct ViewPipelinesConfig {
        params: ConnectParams,
    }
    impl ViewPipelinesConfig {
        pub fn new<Q: IntoConnectParams>(params: Q)
                -> Result<Self, Box<Error + Send + Sync + 'static>>
        {
            let result = ViewPipelinesConfig{
                params: try!(params.into_connect_params()),
            };
            Ok(result)
        }
        fn conn(&self) -> Result<Connection, Box<Error + Send + Sync>> {
            Ok(try!(Connection::connect(self.params.clone(), SslMode::None)))
        }
    }
    impl TViewPipelinesConfig for ViewPipelinesConfig {
        fn pipeline_by_name(&self, name: &str)
                -> Option<PipelineId>
        {
            retry!{{
                let conn = retry_unwrap!(self.conn());
                let sql = r###"
                    SELECT pipeline_id
                    FROM twelvef_config_pipeline
                    WHERE name = $1
                "###;
                let stmt = retry_unwrap!(conn.prepare(&sql));
                let rows = retry_unwrap!(stmt.query(&[&name]));
                let rows = rows.iter();
                let mut rows = rows.map(|row| {
                    PipelineId(row.get::<_, i32>(0))
                });
                rows.next()
            }}
        }
        fn all(&self) -> Vec<(Cow<str>, PipelineId)> {
            retry!{{
                let conn = retry_unwrap!(self.conn());
                let sql = r###"
                    SELECT name, pipeline_id
                    FROM twelvef_config_pipeline
                "###;
                let stmt = retry_unwrap!(conn.prepare(&sql));
                let rows = retry_unwrap!(stmt.query(&[]));
                let rows = rows.iter();
                let rows = rows.map(|row| {
                    (
                        Cow::Owned(row.get::<_, String>(0)),
                        PipelineId(row.get::<_, i32>(1)),
                    )
                });
                let rows = rows.collect();
                rows
            }}
        }
    }
}

// Errors and args definitions.

quick_error! {
    #[derive(Debug)]
    pub enum GithubBuilderError {
        NotGithub {}
        DbConnect(err: Box<Error + Send + Sync + 'static>) {
            from()
        }
        PjConnect(err: Box<Error + Send + Sync + 'static>) {
        }
        GhConnect(err: Box<Error + Send + Sync + 'static>) {
        }
        MissingKey(key: GithubBuilderKey) {}
        InvalidKey(key: GithubBuilderKey) {}
    }
}

#[derive(Debug)]
pub enum GithubBuilderKey {
    UiType,
    CiType,
    VcsType,
    PipelineDb,
    ProjectDb,
    UiGithubDb,
    UiGithubListen,
    UiGithubHost,
    UiGithubToken,
    UiGithubUser,
    UiGithubSecret,
    CiJenkinsListen,
    CiJenkinsHost,
    CiJenkinsUser,
    CiJenkinsToken,
    CiGithubListen,
    CiGithubSecret,
    VcsGithubHost,
    VcsGithubToken,
    VcsGitExecutable,
    VcsGitName,
    VcsGitEmail,
    ViewListen,
    ViewSecret,
}