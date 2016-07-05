// This file is released under the same terms as Rust itself.

use hex::{FromHex, FromHexError, ToHex};
use hyper;
use hyper::client::Client;
use hyper::header::{Accept, Cookie, CookiePair, Headers, Location, SetCookie, UserAgent, qitem};
use hyper::mime::{Mime, TopLevel, SubLevel, Attr, Value};
use hyper::server::{Request, Response};
use hyper::status::StatusCode;
use hyper::uri::RequestUri;
use serde_json::{self, from_reader as json_from_reader};
use std::convert::AsRef;
use url::form_urlencoded;
use util::USER_AGENT;
use util::crypto::{generate_sha1_hmac, verify_sha1_hmac};

pub enum Auth {
    None,
    Github(String, String, String),
}

#[derive(Clone, Copy)]
pub enum AuthRef<'a> {
    None,
    Github(&'a str, &'a str, &'a str),
}

impl<'a> From<&'a Auth> for AuthRef<'a> {
    fn from(a: &'a Auth) -> AuthRef<'a> {
        match *a {
            Auth::None => AuthRef::None,
            Auth::Github(ref app_id, ref app_secret, ref org) =>
                AuthRef::Github(app_id.as_ref(), app_secret.as_ref(), org.as_ref()),
        }
    }
}

pub struct AuthManager<'a> {
    pub auth: AuthRef<'a>,
    pub secret: &'a str,
}

pub enum CheckResult<'a, 'b: 'a, 'c> {
    Authenticated(Request<'a, 'b>, Response<'c>),
    NotAuthenticated,
    Err(CheckResultError),
}

quick_error!{
    #[derive(Debug)]
    pub enum CheckResultError {
        ErrGithubToken(e: hyper::error::Error) {
            cause(e)
        }
        BadGithubTokenResult(e: serde_json::error::Error) {
            cause(e)
        }
        ErrGithubUsername(e: hyper::error::Error) {
            cause(e)
        }
        BadGithubUsernameResult(e: serde_json::error::Error) {
            cause(e)
        }
        ErrGithubOrgCheck(e: hyper::error::Error) {
            cause(e)
        }
        InvalidGithubStateParam(e: FromHexError) {
            cause(e)
        }
        BadGithubStateParam {}
        MissingGithubParam {}
    }
}

impl<'a> AuthManager<'a> {
    pub fn check<'b, 'c, 'd>(
        &self,
        req: Request<'b, 'c>,
        mut res: Response<'d>,
    ) -> CheckResult<'b, 'c, 'd> {
        macro_rules! try_map {
            ($e: expr, $i: ident, $res: ident) => (
                match $e {
                    Ok(x) => x,
                    Err(e) => {
                        *$res.status_mut() = StatusCode::InternalServerError;
                        return CheckResult::Err(CheckResultError::$i(e));
                    }
                }
            )
        }
        match self.auth {
            AuthRef::None => {
                CheckResult::Authenticated(req, res)
            }
            AuthRef::Github(app_id, app_secret, org) => {
                let authed = req.headers.get::<Cookie>()
                    .and_then(|x| {
                        let mut auth = None;
                        let mut name = None;
                        for cookie in x.iter() {
                            match &cookie.name[..] {
                                "auth" => auth = Some(cookie.value.clone()),
                                "name" => name = Some(cookie.value.clone()),
                                n => {
                                    warn!("Unrecognized cookie {}", n);
                                }
                            }
                        }
                        if let (Some(auth), Some(name)) = (auth, name) {
                            Some((auth, name))
                        } else {
                            None
                        }
                    })
                    .map(|(auth, name)| {
                        match Vec::from_hex(&auth) {
                            Ok(auth) => {
                                verify_sha1_hmac(
                                    self.secret.as_bytes(),
                                    name.as_bytes(),
                                    &auth
                                )
                            }
                            Err(e) => {
                                warn!("Bad auth \"{}\": {:?}", auth, e);
                                false
                            }
                        }
                    })
                    .unwrap_or(false);
                if !authed {
                    let path = if let RequestUri::AbsolutePath(ref path) = req.uri {
                        path
                    } else {
                        *res.status_mut() = StatusCode::BadRequest;
                        return CheckResult::NotAuthenticated;
                    };
                    let path = path.as_bytes();
                    if path.len() > 11 && &path[..11] == b"/_gh_login?" {
                        let query = &path[11..];
                        let query = form_urlencoded::parse(query);
                        let mut code = None;
                        let mut state = None;
                        for param in query {
                            match param.0.as_ref() {
                                "state" => state = Some(param.1.into_owned()),
                                "code" => code = Some(param.1.into_owned()),
                                _ => {
                                    warn!(
                                        "Unrecognized Github oAuth param"
                                    );
                                }
                            }
                        }
                        let (code, state) = if let (Some(code), Some(state)) = (code, state) {
                            (code, state)
                        } else {
                            *res.status_mut() = StatusCode::BadRequest;
                            return CheckResult::Err(CheckResultError::MissingGithubParam);
                        };
                        if !verify_sha1_hmac(
                            self.secret.as_bytes(),
                            &req.remote_addr.ip().to_string().as_bytes(),
                            &try_map!(
                                Vec::from_hex(&state),
                                InvalidGithubStateParam,
                                res
                            )
                        ) {
                            *res.status_mut() = StatusCode::BadRequest;
                            return CheckResult::Err(CheckResultError::BadGithubStateParam);
                        }
                        let client = Client::default();
                        // We got successfully authed to GitHub
                        // But we need to check if we're in the org
                        let mut headers = Headers::new();
                        headers.set(UserAgent(USER_AGENT.to_owned()));
                        headers.set(Accept(vec![
                            qitem(Mime(TopLevel::Application, SubLevel::Json,
                                       vec![(Attr::Charset, Value::Utf8)])),
                        ]));
                        let gh_req = form_urlencoded::Serializer::new(String::new())
                            .append_pair("client_id", app_id)
                            .append_pair("client_secret", app_secret)
                            .append_pair("code", &code)
                            .append_pair("state", &state)
                            .finish();
                        let gh_res = try_map!(
                            client
                                .post("https://github.com/login/oauth/access_token")
                                .body(gh_req.as_bytes())
                                .headers(headers)
                                .send(),
                            ErrGithubToken, res
                        );
                        #[derive(Serialize, Deserialize)]
                        struct GithubToken {
                            access_token: String,
                        }
                        let gh_token: GithubToken = try_map!(
                            json_from_reader(gh_res),
                            BadGithubTokenResult, res
                        );
                        let gh_token = gh_token.access_token;
                        // Now that we're actually authed as this user, we need their name.
                        let mut headers = Headers::new();
                        headers.set(UserAgent(USER_AGENT.to_owned()));
                        headers.set(Accept(vec![
                            qitem(Mime(TopLevel::Application, SubLevel::Json,
                                       vec![(Attr::Charset, Value::Utf8)])),
                        ]));
                        let gh_res = try_map!(
                            client
                                .get(&format!("https://api.github.com/user?access_token={}", gh_token))
                                .headers(headers)
                                .send(),
                            ErrGithubUsername, res
                        );
                        #[derive(Serialize, Deserialize)]
                        struct GithubUser {
                            login: String,
                        }
                        let gh_user: GithubUser = try_map!(
                            json_from_reader(gh_res),
                            BadGithubUsernameResult, res
                        );
                        let gh_user = gh_user.login;
                        // Note: the user may need to request (or give) this app org perms to do this.
                        let mut headers = Headers::new();
                        headers.set(UserAgent(USER_AGENT.to_owned()));
                        headers.set(Accept(vec![
                            qitem(Mime(TopLevel::Application, SubLevel::Json,
                                       vec![(Attr::Charset, Value::Utf8)])),
                        ]));
                        let gh_res = try_map!(
                            client
                                .get(&format!("https://api.github.com/org/{}/members/{}?access_token={}", org, gh_user, gh_token))
                                .headers(headers)
                                .send(),
                            ErrGithubOrgCheck, res
                        );
                        if gh_res.status.is_success() {
                            // Awesome: we're authenticated!
                            let auth = generate_sha1_hmac(
                                self.secret.as_bytes(),
                                gh_user.as_bytes()
                            ).to_hex();
                            res.headers_mut().set(SetCookie(vec![
                                CookiePair::new("auth".to_owned(), auth),
                                CookiePair::new("name".to_owned(), gh_user)
                            ]));
                            res.headers_mut().set(Location(".".to_owned()));
                            *res.status_mut() = StatusCode::Found;
                        } else {
                            warn!("Got Github user that's not member: {:?}", gh_user);
                            *res.status_mut() = StatusCode::Forbidden;
                        }
                    } else {
                        let state = generate_sha1_hmac(
                            self.secret.as_bytes(),
                            req.remote_addr.ip().to_string().as_bytes()
                        ).to_hex();
                        res.headers_mut().set(Location(
                            format!(
                                "https://github.com/login/oauth/authorize?client_id={}&state={}&scope=read:org",
                                app_id,
                                state
                            )
                        ));
                        *res.status_mut() = StatusCode::Found;
                    }
                    CheckResult::NotAuthenticated
                } else {
                    CheckResult::Authenticated(req, res)
                }
            }
        }
    }
}