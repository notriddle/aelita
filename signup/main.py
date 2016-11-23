# This file is released under the same terms as Rust iself.

from flask import Flask, g, redirect, render_template, request, session
from flask import flash, url_for
from flask_github import GitHub
import json
import os
import requests
from sqlalchemy import create_engine, Column, Integer, String, Boolean
from sqlalchemy.orm import scoped_session, sessionmaker
from sqlalchemy.ext.declarative import declarative_base


app = Flask(__name__)


if 'SENTRY_DSN' in os.environ:
    from raven.contrib.flask import Sentry
    sentry = Sentry(app)


app.config['GITHUB_CLIENT_ID'] = os.environ['AELITA_GITHUB_CLIENT_ID']
app.config['GITHUB_CLIENT_SECRET'] = os.environ['AELITA_GITHUB_CLIENT_SECRET']
app.config['BOT_USERNAME'] = os.environ['AELITA_BOT_USERNAME']
app.config['BOT_BASEURL'] = os.environ['AELITA_BOT_BASEURL']
app.config['BOT_DBURI'] = os.environ['AELITA_BOT_DBURI']
app.config['BOT_ACCESS_TOKEN'] = os.environ['AELITA_BOT_ACCESS_TOKEN']
app.config['BOT_NOTICE_SECRET'] = os.environ['AELITA_BOT_NOTICE_SECRET']
app.config['BOT_STATUS_SECRET'] = os.environ['AELITA_BOT_STATUS_SECRET']
app.secret_key = os.environ['AELITA_VIEW_SECRET']
github = GitHub(app)
engine = create_engine(app.config['BOT_DBURI'])
db_session = scoped_session(
    sessionmaker(
        autocommit=False,
        autoflush=False,
        bind=engine,
    )
)


Base = declarative_base()
Base.query = db_session.query_property()


class User(Base):
    __tablename__ = 'signup_users'

    user_id = Column(Integer, primary_key=True)
    username = Column(String(200))
    github_access_token = Column(String(200))
    invite_count = Column(Integer, default=3)

    def __init__(self, github_access_token):
        self.github_access_token = github_access_token


class Invited(Base):
    __tablename__ = 'signup_invited'

    username = Column(String(200), primary_key=True)

    def __init__(self, username):
        self.username = username


class Pipeline(Base):
    __tablename__ = 'twelvef_config_pipeline'

    pipeline_id = Column(Integer, primary_key=True)
    name = Column(String(200))

    def __init__(self, pipeline_id, name):
        self.pipeline_id = pipeline_id
        self.name = name


class GithubProjects(Base):
    __tablename__ = 'twelvef_github_projects'

    pipeline_id = Column(Integer, primary_key=True)
    try_pipeline_id = Column(Integer, nullable=True)
    owner = Column(String(200))
    repo = Column(String(200))

    def __init__(self, pipeline_id, try_pipeline_id, owner, repo):
        self.pipeline_id = pipeline_id
        self.try_pipeline_id = try_pipeline_id
        self.owner = owner
        self.repo = repo


class GithubStatusPipelines(Base):
    __tablename__ = 'twelvef_github_status_pipelines'

    ci_id = Column(Integer, primary_key=True)
    owner = Column(String(200))
    repo = Column(String(200))
    context = Column(String(200))

    def __init__(self, ci_id, owner, repo, context):
        self.ci_id = ci_id
        self.owner = owner
        self.repo = repo
        self.context = context


class PipelineCi(Base):
    __tablename__ = 'twelvef_config_pipeline_ci'

    pipeline_id = Column(Integer, primary_key=True)
    ci_id = Column(Integer)

    def __init__(self, pipeline_id, ci_id):
        self.ci_id = ci_id
        self.pipeline_id = pipeline_id


class GithubGitPipelines(Base):
    __tablename__ = 'twelvef_github_git_pipelines'

    pipeline_id = Column(Integer, primary_key=True)
    owner = Column(String(200))
    repo = Column(String(200))
    master_branch = Column(String(200))
    staging_branch = Column(String(200))
    push_to_master = Column(Boolean)

    def __init__(self, pipeline_id, owner, repo):
        self.pipeline_id = pipeline_id
        self.owner = owner
        self.repo = repo
        self.master_branch = "master"
        self.staging_branch = "staging"
        self.push_to_master = True


Base.metadata.create_all(bind=engine)


@app.before_request
def before_request():
    g.user = None


@app.teardown_appcontext
def shutdown_session(exception=None):
    db_session.remove()
    g.user = None


def get_user():
    if g.user is None and 'user_id' in session:
        g.user = User.query.get(session['user_id'])
    return g.user


@github.access_token_getter
def token_getter():
    user = get_user()
    if user is not None:
        return user.github_access_token


@app.route('/')
def index():
    if 'user_id' not in session:
        return render_template('index.html')
    else:
        return redirect(url_for('manage'))


@app.route('/login', methods=['POST'])
def login():
    return github.authorize(scope="user,repo")


@app.route('/logout', methods=['POST'])
def logout():
    session.pop('user_id', None)
    return redirect(url_for('index'))


@app.route('/github-callback')
@github.authorized_handler
def authorized(oauth_token):
    if oauth_token is None:
        flash("Authorization failed.")
        return redirect(url_for('index'))
    g.user = User(oauth_token)
    username = github.get('user')['login']
    g.user.username = username
    user = User.query.filter_by(username=username).first()
    if user is None:
        invite = Invited.query.filter_by(username=username).first()
        if invite is None:
            flash("This service is invite-only")
            return redirect(url_for('index'))
        user = db_session.merge(g.user)
        g.user = user
        db_session.add(user)
        db_session.commit()
    elif user.github_access_token != oauth_token:
        user.github_access_token = oauth_token
        user = db_session.merge(user)
        db_session.commit()
    session['user_id'] = user.user_id
    return redirect(url_for('manage'))


@app.route('/manage', methods=['GET', 'POST'])
def manage():
    user = get_user()
    if user is None:
        flash("Please log in")
        return logout()
    all_repos = github.get('user/repos?visibility=public', all_pages=True)
    repo_defs = []
    owner_set = {}
    edit = None
    default_context = 'continuous-integration/travis-ci/push'
    filter_owner = request.args.get('owner')
    if filter_owner is None and request.method == "GET" and \
            request.args.get("edit") is None:
        filter_owner = user.username
    elif filter_owner == "-":
        filter_owner = None
    for repo in all_repos:
        if not repo['permissions']['admin']:
            continue
        owner_set[repo['owner']['login']] = repo['owner']
        if filter_owner is not None and filter_owner != repo['owner']['login']:
            continue
        on_repo = GithubProjects.query \
            .filter_by(owner=repo['owner']['login'],repo=repo['name']) \
            .first()
        repo_def = {
            "owner": repo['owner']['login'],
            "repo": repo['name'],
            "name": repo['full_name'],
            "id": repo['id'],
            "present": on_repo is not None,
        }
        if request.method == 'POST':
            if 'add' in request.form and \
                    int(request.form['add']) == repo['id'] and \
                    on_repo is None:
                return add_repo(repo, default_context)
            elif 'remove' in request.form and \
                    int(request.form['remove']) == repo['id'] and \
                    on_repo is not None:
                return remove_repo(repo, on_repo)
            elif 'edit' in request.form and \
                    int(request.form['edit']) == repo['id'] and \
                    on_repo is not None:
                return edit_repo(on_repo)
        repo_defs.append(repo_def)
        if on_repo is not None:
            if request.args.get('edit') and \
                    request.args.get('edit') == str(repo['id']):
                edit = repo_def
                cis = PipelineCi.query.filter_by(pipeline_id=pipeline_id)
                contexts = ""
                for ci in cis:
                    on_status = GithubStatusPipelines.query \
                        .filter_by(ci_id=ci.ci_id) \
                        .first()
                    if contexts != "": contexts = contexts + ","
                    contexts = contexts + on_status.context
                on_git = GithubGitPipelines.query \
                    .filter_by(pipeline_id=on_repo.pipeline_id) \
                    .first()
                edit['contexts'] = contexts
                edit['master_branch'] = on_git.master_branch
                edit['staging_branch'] = on_git.staging_branch
    # Order: <me> <other users> <orgs>
    def owner_key(item):
        if item['login'] == user.username:
            return "0"
        elif item['type'] == 'User':
            return "0" + item['login']
        else:
            return "1" + item['login']
    if len(owner_set) > 1:
        owner_defs = sorted(
            (item[1] for item in owner_set.items()),
            key=owner_key
        )
    return render_template(
        'manage.html',
        username=user.username,
        repo_defs=repo_defs,
        owner_defs=owner_defs,
        filter_owner=filter_owner,
        edit=edit,
        invite_count=user.invite_count,
        base_url=app.config['BOT_BASEURL'],
    )

def add_repo(repo, context):
    user = get_user()
    # Add repository to our database
    pipeline = Pipeline(None, repo['full_name'])
    db_session.add(pipeline)
    status = GithubStatusPipelines(
        None,
        repo['owner']['login'],
        repo['name'],
        context
    )
    db_session.add(status)
    db_session.flush()
    pipeline_id = pipeline.pipeline_id
    ci_id = status.ci_id
    ci = PipelineCi(pipeline_id, ci_id)
    db_session.add(ci)
    on_repo = GithubProjects(
        pipeline_id,
        None,
        repo['owner']['login'],
        repo['name']
    )
    db_session.add(on_repo)
    git = GithubGitPipelines(
        pipeline_id,
        repo['owner']['login'],
        repo['name']
    )
    db_session.add(git)
    db_session.commit()
    # Add our account as a collaborator on Github
    headers = {
        "Content-Type": "application/vnd.github.swamp-thing-preview+json",
    }
    invite = github.request(
        'PUT',
        'repos/' + repo['full_name'] + '/collaborators/' + \
            app.config['BOT_USERNAME'],
        headers=headers,
    )
    headers = {
        "Authorization": "token " + app.config['BOT_ACCESS_TOKEN'],
        "Content-Type": "application/vnd.github.swamp-thing-preview+json",
    }
    requests.request(
        'PATCH',
        invite.url,
        headers=headers,
    )
    # Add our user as a member of the organization, if we aren't already
    if repo['owner']['type'] == 'Organization':
        add_me_to_org(repo['owner']['login'])
    # Add our webhooks
    github.post(
        'repos/' + repo['full_name'] + '/hooks',
        {
            "name": "web",
            "active": True,
            "config": {
                "url": app.config['BOT_BASEURL'] + '/github-notice',
                "content_type": "json",
                "secret": app.config['BOT_NOTICE_SECRET']
            },
            "events": [ "issue_comment", "pull_request", "team_add" ]
        }
    )
    github.post(
        'repos/' + repo['full_name'] + '/hooks',
        {
            "name": "web",
            "active": True,
            "config": {
                "url": app.config['BOT_BASEURL'] + '/github-status',
                "content_type": "json",
                "secret": app.config['BOT_STATUS_SECRET']
            },
            "events": [ "status" ]
        }
    )
    flash("Added successfully")
    return redirect(url_for('manage'))

def remove_repo(repo, on_repo):
    user = get_user()
    # Remove from our database
    pipeline_id = on_repo.pipeline_id
    db_session.delete(on_repo)
    pipeline = Pipeline.query.filter_by(pipeline_id=pipeline_id).first()
    if not pipeline is None: db_session.delete(pipeline)
    cis = PipelineCi.query.filter_by(pipeline_id=pipeline_id)
    for ci in cis:
        status = GithubStatusPipelines.query \
                .filter_by(ci_id=ci.ci_id) \
                .first();
        if not status is None: db_session.delete(status)
        db_session.delete(ci)
    git = GithubGitPipelines.query \
            .filter_by(pipeline_id=pipeline_id) \
            .first();
    if not git is None: db_session.delete(git)
    db_session.commit()
    # Remove our collaboratorship
    github.raw_request(
        'DELETE',
        'repos/' + repo['full_name'] + '/collaborators/' + \
            app.config['BOT_USERNAME']
    )
    # Remove our webhooks
    hooks = github.get(
        'repos/' + repo['full_name'] + '/hooks'
    )
    my_urls = {
        app.config['BOT_BASEURL'] + '/github-notice',
        app.config['BOT_BASEURL'] + '/github-status',
    }
    for webhook in hooks:
        if webhook['name'] == 'web' and webhook['config']['url'] in my_urls:
            github.request(
                'DELETE',
                'repos/' + repo['full_name'] + '/hooks/' + str(webhook['id'])
            )
    flash("Deleted successfully")
    return redirect(url_for('manage'))


def edit_repo(project):
    user = get_user()
    pipeline_id = project.pipeline_id
    cis = PipelineCi.query.filter_by(pipeline_id=pipeline_id)
    for ci in cis:
        db_session.delete(ci)
        status = GithubStatusPipelines.query \
                .filter_by(ci_id=ci.ci_id) \
                .first();
        db_session.delete(status)
    for context in request.form['contexts'].split(",").replace(" ",""):
        status = GithubStatusPipelines(None, project.owner, project.repo, context)
        db_session.add(status)
        db_session.flush()
        ci = PipelineCi(pipeline_id, status.ci_id)
        db_session.add(ci)
    git = GithubGitPipelines.query \
            .filter_by(pipeline_id=pipeline_id) \
            .first();
    git.master_branch = request.form['master_branch']
    git.staging_branch = request.form['staging_branch']
    db_session.commit()
    flash("Saved successfully")
    return redirect(url_for('manage'))


def add_me_to_org(login):
    invite = github.request(
        'PUT',
        'orgs/' + login + '/memberships/' + \
            app.config['BOT_USERNAME'],
    )
    if invite['state'] != "active":
        headers = {
            "Authorization": "token " + app.config['BOT_ACCESS_TOKEN'],
            "Content-Type": "application/json",
        }
        requests.request(
            'PATCH',
            "https://api.github.com/user/memberships/orgs/" + login,
            data=json.dumps({
                "state": "active"
            }),
            headers=headers
        ).raise_for_status()


@app.route('/invite', methods=['POST'])
def invite():
    user = get_user()
    if user is None:
        flash("Please log in")
        return redirect(url_for('index'))
    if user.invite_count <= 0:
        flash("You're out of invites")
        return redirect(url_for('manage'))
    invite = Invited.query.filter_by(username=request.form['username'])
    if invite.first() is not None:
        flash("This person is already invited")
        return redirect(url_for('manage'))
    invite = Invited(request.form['username'])
    db_session.add(invite)
    user.invite_count = user.invite_count - 1
    db_session.commit()
    flash("Invitation recorded; now let them know!")
    return redirect(url_for('manage'))
