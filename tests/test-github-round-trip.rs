// This file is released under the same terms as Rust itself.

//! Test the entire system front-to-back.
//! This is only a test for the happy path, and it's not thorough,
//! but what it lacks in thorough, it makes up for in broad.

#![feature(proc_macro)]

extern crate crossbeam;
extern crate env_logger;
extern crate hex;
extern crate hyper;
#[macro_use] extern crate lazy_static;
#[macro_use] extern crate log;
extern crate openssl;
extern crate postgres;
#[macro_use] extern crate quick_error;
extern crate regex;
extern crate rusqlite;
extern crate serde;
extern crate serde_json;
#[macro_use] extern crate serde_derive;
extern crate toml;
extern crate url;
extern crate void;

use hex::ToHex;
use hyper::buffer::BufReader;
use hyper::client::Client;
use hyper::header::{Authorization, Basic, Headers};
use hyper::method::Method;
use hyper::net::{HttpListener, NetworkListener, NetworkStream};
use hyper::server::{Request, Response};
use hyper::status::StatusCode;
use hyper::uri::RequestUri;
use std::fs::File;
use std::io::{BufWriter, Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::process::Command;
use std::sync::{Mutex, Once, ONCE_INIT};
use std::thread;
use std::time;

const EXECUTABLE: &'static str = "target/debug/aelita";

lazy_static!{
    static ref ONE_AT_A_TIME: Mutex<()> = Mutex::new(());
}

static START: Once = ONCE_INIT;

fn single_request<T, H>(listener: &mut HttpListener, h: H) -> T
    where H: FnOnce(Request, Response) -> T
{
    let mut stream = listener.accept().unwrap();
    let addr = stream.peer_addr()
        .expect("webhook client address");
    let mut stream_clone = stream.clone();
    let mut buf_read = BufReader::new(
        &mut stream_clone as &mut NetworkStream
    );
    let mut buf_write = BufWriter::new(&mut stream);
    let req = Request::new(&mut buf_read, addr)
        .expect("webhook Request");
    let mut head = Headers::new();
    let res = Response::new(&mut buf_write, &mut head);
    h(req, res)
}

#[test]
fn one_item_github_round_trip() {
    let _lock = ONE_AT_A_TIME.lock();
    START.call_once(|| env_logger::init().unwrap());

    if !Path::new(EXECUTABLE).exists() {
        panic!("Integration tests require the executable to be built.");
    }

    let mut github_server = HttpListener::new(&"localhost:9011").unwrap();
    let mut jenkins_server = HttpListener::new(&"localhost:9012").unwrap();

    Command::new("/bin/tar")
        .current_dir("./tests/")
        .arg("-xvf")
        .arg("cache.tar.gz")
        .output()
        .unwrap();

    Command::new("/bin/rm")
        .current_dir("./tests/")
        .arg("db.sqlite")
        .output()
        .unwrap();

    let executable = Path::new(EXECUTABLE).canonicalize().unwrap();
    let mut aelita = Command::new(executable)
        .current_dir("./tests/")
        .arg("test-github-round-trip.toml")
        .spawn()
        .unwrap();

    info!("Wait a sec for it to finish starting.");
    thread::sleep(time::Duration::new(2, 0));

    info!("Pull request comes into existance.");
    let http_client = Client::new();
    let mut http_headers = Headers::new();
    http_headers.set_raw("X-Github-Event", vec![b"pull_request".to_vec()]);
    let body = concat!(r#" { "#,
        r#" "action":"opened", "#,
        r#" "repository":{ "#,
            r#" "name":"testp", "#,
            r#" "owner":{"login":"AelitaBot","type":"User"} "#,
        r#" }, "#,
        r#" "pull_request":{ "#,
            r#" "title":"HA!", "#,
            r#" "html_url":"http://github.com/testu/testp/pull_request/1", "#,
            r#" "state":"opened", "#,
            r#" "number":1, "#,
            r#" "head":{ "#,
                r#" "sha":"55016813274e906e4cbfed97be83e19e6cd93d91" "#,
            r#" } "#,
        r#" } "#,
    r#" } "#);
    http_headers.set_raw("X-Hub-Signature", vec![
        format!("sha1={}", openssl::crypto::hmac::hmac(
            openssl::crypto::hash::Type::SHA1,
            "ME_SECRET_LOL".as_bytes(),
            body.as_bytes(),
        ).to_hex()).into_bytes()
    ]);
    http_client.post("http://localhost:9001")
        .body(body.as_bytes())
        .headers(http_headers)
        .send()
        .unwrap();

    info!("User posts comment to mark pull request reviewed.");
    let http_client = Client::new();
    let mut http_headers = Headers::new();
    http_headers.set_raw("X-Github-Event", vec![b"issue_comment".to_vec()]);
    let body = concat!(r#" { "#,
        r#" "issue":{ "#,
            r#" "number":1, "#,
            r#" "title":"My PR!", "#,
            r#" "body":"Test", "#,
            r#" "pull_request":{ "#,
                r#" "html_url":"http://github.com/testu/testp/pull_request/1" "#,
            r#" }, "#,
            r#" "state":"opened", "#,
            r#" "user":{ "#,
                r#" "login":"testu", "#,
                r#" "type":"User" "#,
            r#" } "#,
        r#" }, "#,
        r#" "comment":{ "#,
            r#" "user":{ "#,
                r#" "login":"testu", "#,
                r#" "type":"User" "#,
            r#" }, "#,
            r#" "body":"@AelitaBot r+" "#,
        r#" }, "#,
        r#" "repository":{ "#,
            r#" "name":"testp", "#,
            r#" "owner":{ "#,
                r#" "login":"AelitaBot", "#,
                r#" "type":"User" "#,
            r#" } "#,
        r#" } "#,
    r#" } "#);
    http_headers.set_raw("X-Hub-Signature", vec![
        format!("sha1={}", openssl::crypto::hmac::hmac(
            openssl::crypto::hash::Type::SHA1,
            "ME_SECRET_LOL".as_bytes(),
            body.as_bytes(),
        ).to_hex()).into_bytes()
    ]);
    http_client.post("http://localhost:9001")
        .body(body)
        .headers(http_headers)
        .send()
        .unwrap();

    info!("Aelita asks if we're an organization. We're not, for this test.");
    single_request(&mut github_server, |req, mut res| {
        assert_eq!(
            req.uri,
            RequestUri::AbsolutePath("/repos/AelitaBot/testp".to_owned())
        );
        assert_eq!(
            &req.headers.get_raw("Authorization").unwrap()[0][..],
            b"token MY_PERSONAL_ACCESS_TOKEN"
        );
        *res.status_mut() = StatusCode::Ok;
        res.send(concat!(r#" { "#,
            r#" "name":"testp", "#,
            r#" "owner":{"login":"AelitaBot","type":"User"} "#,
            r#" } "#).as_bytes()).unwrap();
    });

    info!("Aelita checks if user has permission to do that.");
    single_request(&mut github_server, |req, mut res| {
        let path = "/repos/AelitaBot/testp/collaborators/testu".to_owned();
        assert_eq!(
            req.uri,
            RequestUri::AbsolutePath(path)
        );
        assert_eq!(
            &req.headers.get_raw("Authorization").unwrap()[0][..],
            b"token MY_PERSONAL_ACCESS_TOKEN"
        );
        *res.status_mut() = StatusCode::NoContent;
        res.send(&[]).unwrap();
    });

    info!("Aelita sends build trigger.");
    single_request(&mut jenkins_server, |req, mut res| {
        let path = "/job/testp/build?token=MY_BUILD_TOKEN".to_owned();
        assert_eq!(
            req.uri,
            RequestUri::AbsolutePath(path)
        );
        assert_eq!(
            req.headers.get::<Authorization<Basic>>().unwrap().0,
            Basic{
                username: "AelitaBot".to_owned(),
                password: Some("MY_JENKINS_API_TOKEN".to_owned()),
            }
        );
        *res.status_mut() = StatusCode::NoContent;
        res.send(&[]).unwrap();
    });

    info!("Aelita does the merge.");
    let mut commit_string = String::new();
    File::open(Path::new("tests/cache/origin/.git/refs/heads/staging"))
        .unwrap()
        .read_to_string(&mut commit_string)
        .unwrap();
    commit_string = commit_string.replace("\n", "").replace("\r", "");

    info!("Jenkins sends start notification to Aelita.");
    let mut tcp_client = TcpStream::connect("localhost:9002").unwrap();
    tcp_client.write(
        r#"{"name":"testp","build":{"phase":"STARTED","full_url":"http://jenkins.com/job/1/","scm":{"commit":"CMMT"}}}"#
            .replace("CMMT", &commit_string)
            .as_bytes()
    ).unwrap();
    drop(tcp_client);

    info!("Jenkins sends finished notification to Aelita.");
    let mut tcp_client = TcpStream::connect("localhost:9002").unwrap();
    tcp_client.write(
        r#"{"name":"testp","build":{"phase":"COMPLETED","status":"SUCCESS","full_url":"http://jenkins.com/job/1/","scm":{"commit":"CMMT"}}}"#
            .replace("CMMT", &commit_string)
            .as_bytes()
    ).unwrap();
    drop(tcp_client);

    info!("Wait a sec for it to finish pushing.");
    thread::sleep(time::Duration::new(2, 0));

    info!("Aelita fast-forwards master.");
    let mut master_string = String::new();
    File::open(Path::new("tests/cache/origin/.git/refs/heads/master"))
        .unwrap()
        .read_to_string(&mut master_string)
        .unwrap();
    master_string = master_string.replace("\n", "").replace("\r", "");
    assert!(master_string != "e16d1eca074ae29ac1812e14316e96f3117d0675");

    aelita.kill().unwrap();
}

#[test]
fn one_item_team_github_round_trip() {
    let _lock = ONE_AT_A_TIME.lock();
    START.call_once(|| env_logger::init().unwrap());

    if !Path::new(EXECUTABLE).exists() {
        panic!("Integration tests require the executable to be built.");
    }

    let mut github_server = HttpListener::new(&"localhost:9011").unwrap();
    let mut jenkins_server = HttpListener::new(&"localhost:9012").unwrap();

    Command::new("/bin/tar")
        .current_dir("./tests/")
        .arg("-xvf")
        .arg("cache.tar.gz")
        .output()
        .unwrap();

    Command::new("/bin/rm")
        .current_dir("./tests/")
        .arg("db.sqlite")
        .output()
        .unwrap();

    let executable = Path::new(EXECUTABLE).canonicalize().unwrap();
    let mut aelita = Command::new(executable.clone())
        .current_dir("./tests/")
        .arg("test-github-round-trip.toml")
        .spawn()
        .unwrap();

    info!("Wait a sec for it to finish writing.");
    thread::sleep(time::Duration::new(1, 0));

    info!("Restart Aelita, forcing it to use the persistent team cache.");
    aelita.kill().unwrap();
    thread::sleep(time::Duration::new(1, 0));
    let mut aelita = Command::new(executable)
        .current_dir("./tests/")
        .arg("test-github-round-trip.toml")
        .spawn()
        .unwrap();

    info!("Wait a sec for it to finish starting.");
    thread::sleep(time::Duration::new(2, 0));

    info!("Pull request comes into existance.");
    let http_client = Client::new();
    let mut http_headers = Headers::new();
    http_headers.set_raw("X-Github-Event", vec![b"pull_request".to_vec()]);
    let body = concat!(r#" { "#,
        r#" "action":"opened", "#,
        r#" "repository":{ "#,
            r#" "name":"testp", "#,
            r#" "owner":{"login":"AelitaBot","type":"User"} "#,
        r#" }, "#,
        r#" "pull_request":{ "#,
            r#" "title":"HA!", "#,
            r#" "html_url":"http://github.com/testu/testp/pull_request/1", "#,
            r#" "state":"opened", "#,
            r#" "number":1, "#,
            r#" "head":{ "#,
                r#" "sha":"55016813274e906e4cbfed97be83e19e6cd93d91" "#,
            r#" } "#,
        r#" } "#,
    r#" } "#);
    http_headers.set_raw("X-Hub-Signature", vec![
        format!("sha1={}", openssl::crypto::hmac::hmac(
            openssl::crypto::hash::Type::SHA1,
            "ME_SECRET_LOL".as_bytes(),
            body.as_bytes(),
        ).to_hex()).into_bytes()
    ]);
    http_client.post("http://localhost:9001")
        .body(body.as_bytes())
        .headers(http_headers)
        .send()
        .unwrap();

    info!("User posts comment to mark pull request reviewed.");
    let http_client = Client::new();
    let mut http_headers = Headers::new();
    http_headers.set_raw("X-Github-Event", vec![b"issue_comment".to_vec()]);
    let body = concat!(r#" { "#,
        r#" "issue":{ "#,
            r#" "number":1, "#,
            r#" "title":"My PR!", "#,
            r#" "body":"Test", "#,
            r#" "pull_request":{ "#,
                r#" "html_url":"http://github.com/testu/testp/pull_request/1" "#,
            r#" }, "#,
            r#" "state":"opened", "#,
            r#" "user":{ "#,
                r#" "login":"testu", "#,
                r#" "type":"User" "#,
            r#" } "#,
        r#" }, "#,
        r#" "comment":{ "#,
            r#" "user":{ "#,
                r#" "login":"testu", "#,
                r#" "type":"User" "#,
            r#" }, "#,
            r#" "body":"@AelitaBot r+" "#,
        r#" }, "#,
        r#" "repository":{ "#,
            r#" "name":"testp", "#,
            r#" "owner":{ "#,
                r#" "login":"AelitaBot", "#,
                r#" "type":"User" "#,
            r#" } "#,
        r#" } "#,
    r#" } "#);
    http_headers.set_raw("X-Hub-Signature", vec![
        format!("sha1={}", openssl::crypto::hmac::hmac(
            openssl::crypto::hash::Type::SHA1,
            "ME_SECRET_LOL".as_bytes(),
            body.as_bytes(),
        ).to_hex()).into_bytes()
    ]);
    http_client.post("http://localhost:9001")
        .body(body)
        .headers(http_headers)
        .send()
        .unwrap();

    info!("Aelita asks if we're an organization. We are, for this test.");
    single_request(&mut github_server, |req, mut res| {
        assert_eq!(
            req.uri,
            RequestUri::AbsolutePath("/repos/AelitaBot/testp".to_owned())
        );
        assert_eq!(
            &req.headers.get_raw("Authorization").unwrap()[0][..],
            b"token MY_PERSONAL_ACCESS_TOKEN"
        );
        *res.status_mut() = StatusCode::Ok;
        res.send(concat!(r#" { "#,
            r#" "name":"testp", "#,
            r#" "owner":{"login":"AelitaBot","type":"Organization"} "#,
            r#" } "#).as_bytes()).unwrap();
    });

    info!("Aelita gets a list of teams.");
    single_request(&mut github_server, |req, mut res| {
        assert_eq!(
            req.uri,
            RequestUri::AbsolutePath("/orgs/AelitaBot/teams".to_owned())
        );
        assert_eq!(
            &req.headers.get_raw("Authorization").unwrap()[0][..],
            b"token MY_PERSONAL_ACCESS_TOKEN"
        );
        *res.status_mut() = StatusCode::Ok;
        res.send(concat!(r#" [ "#,
            r#" { "id":1, "slug":"Potato" } "#,
            r#" ] "#).as_bytes()).unwrap();
    });

    info!("Aelita checks if Potato has write permission. It does.");
    single_request(&mut github_server, |req, mut res| {
        assert_eq!(
            req.uri,
            RequestUri::AbsolutePath(
                "/teams/1/repos/AelitaBot/testp".to_owned()
            )
        );
        assert_eq!(
            &req.headers.get_raw("Authorization").unwrap()[0][..],
            b"token MY_PERSONAL_ACCESS_TOKEN"
        );
        *res.status_mut() = StatusCode::Ok;
        res.send(concat!(r#" { "#,
            r#" "permissions": { "pull":true,"push":true,"admin":false } "#,
            r#" } "#).as_bytes()).unwrap();
    });

    info!("Aelita checks if user is member of Potato.");
    single_request(&mut github_server, |req, mut res| {
        let path = "/teams/1/members/testu".to_owned();
        assert_eq!(
            req.uri,
            RequestUri::AbsolutePath(path)
        );
        assert_eq!(
            &req.headers.get_raw("Authorization").unwrap()[0][..],
            b"token MY_PERSONAL_ACCESS_TOKEN"
        );
        *res.status_mut() = StatusCode::NoContent;
        res.send(&[]).unwrap();
    });

    info!("Aelita sends build trigger.");
    single_request(&mut jenkins_server, |req, mut res| {
        let path = "/job/testp/build?token=MY_BUILD_TOKEN".to_owned();
        assert_eq!(
            req.uri,
            RequestUri::AbsolutePath(path)
        );
        assert_eq!(
            req.headers.get::<Authorization<Basic>>().unwrap().0,
            Basic{
                username: "AelitaBot".to_owned(),
                password: Some("MY_JENKINS_API_TOKEN".to_owned()),
            }
        );
        *res.status_mut() = StatusCode::NoContent;
        res.send(&[]).unwrap();
    });

    info!("Aelita does the merge.");
    let mut commit_string = String::new();
    File::open(Path::new("tests/cache/origin/.git/refs/heads/staging"))
        .unwrap()
        .read_to_string(&mut commit_string)
        .unwrap();
    commit_string = commit_string.replace("\n", "").replace("\r", "");

    info!("Jenkins sends start notification to Aelita.");
    let mut tcp_client = TcpStream::connect("localhost:9002").unwrap();
    tcp_client.write(
        r#"{"name":"testp","build":{"phase":"STARTED","full_url":"http://jenkins.com/job/1/","scm":{"commit":"CMMT"}}}"#
            .replace("CMMT", &commit_string)
            .as_bytes()
    ).unwrap();
    drop(tcp_client);

    info!("Jenkins sends finished notification to Aelita.");
    let mut tcp_client = TcpStream::connect("localhost:9002").unwrap();
    tcp_client.write(
        r#"{"name":"testp","build":{"phase":"COMPLETED","status":"SUCCESS","full_url":"http://jenkins.com/job/1/","scm":{"commit":"CMMT"}}}"#
            .replace("CMMT", &commit_string)
            .as_bytes()
    ).unwrap();
    drop(tcp_client);

    info!("Wait a sec for it to finish pushing.");
    thread::sleep(time::Duration::new(2, 0));

    info!("Aelita fast-forwards master.");
    let mut master_string = String::new();
    File::open(Path::new("tests/cache/origin/.git/refs/heads/master"))
        .unwrap()
        .read_to_string(&mut master_string)
        .unwrap();
    master_string = master_string.replace("\n", "").replace("\r", "");
    assert!(master_string != "e16d1eca074ae29ac1812e14316e96f3117d0675");

    aelita.kill().unwrap();
}

#[test]
fn one_item_github_round_trip_status() {
    let _lock = ONE_AT_A_TIME.lock();
    START.call_once(|| env_logger::init().unwrap());

    if !Path::new(EXECUTABLE).exists() {
        panic!("Integration tests require the executable to be built.");
    }

    let mut github_server = HttpListener::new(&"localhost:9011").unwrap();

    Command::new("/bin/tar")
        .current_dir("./tests/")
        .arg("-xvf")
        .arg("cache.tar.gz")
        .output()
        .unwrap();

    Command::new("/bin/rm")
        .current_dir("./tests/")
        .arg("db.sqlite")
        .output()
        .unwrap();

    let executable = Path::new(EXECUTABLE).canonicalize().unwrap();
    let mut aelita = Command::new(executable)
        .current_dir("./tests/")
        .arg("test-github-round-trip-status.toml")
        .spawn()
        .unwrap();

    info!("Wait a sec for it to finish starting.");
    thread::sleep(time::Duration::new(2, 0));

    info!("Pull request comes into existance.");
    let http_client = Client::new();
    let mut http_headers = Headers::new();
    http_headers.set_raw("X-Github-Event", vec![b"pull_request".to_vec()]);
    let body = concat!(r#" { "#,
        r#" "action":"opened", "#,
        r#" "repository":{ "#,
            r#" "name":"testp", "#,
            r#" "owner":{"login":"AelitaBot","type":"User"} "#,
        r#" }, "#,
        r#" "pull_request":{ "#,
            r#" "title":"HA!", "#,
            r#" "html_url":"http://github.com/testu/testp/pull_request/1", "#,
            r#" "state":"opened", "#,
            r#" "number":1, "#,
            r#" "head":{ "#,
                r#" "sha":"55016813274e906e4cbfed97be83e19e6cd93d91" "#,
            r#" } "#,
        r#" } "#,
    r#" } "#);
    http_headers.set_raw("X-Hub-Signature", vec![
        format!("sha1={}", openssl::crypto::hmac::hmac(
            openssl::crypto::hash::Type::SHA1,
            "ME_SECRET_LOL".as_bytes(),
            body.as_bytes(),
        ).to_hex()).into_bytes()
    ]);
    http_client.post("http://localhost:9001")
        .body(body)
        .headers(http_headers)
        .send()
        .unwrap();

    info!("User posts comment to mark pull request reviewed.");
    let http_client = Client::new();
    let mut http_headers = Headers::new();
    http_headers.set_raw("X-Github-Event", vec![b"issue_comment".to_vec()]);
    let body = concat!(r#" { "#,
        r#" "issue":{ "#,
            r#" "number":1, "#,
            r#" "title":"My PR!", "#,
            r#" "body":"Test", "#,
            r#" "pull_request":{ "#,
                r#" "html_url":"http://github.com/testu/testp/pull_request/1" "#,
            r#" }, "#,
            r#" "state":"opened", "#,
            r#" "user":{ "#,
                r#" "login":"testu", "#,
                r#" "type":"User" "#,
            r#" } "#,
        r#" }, "#,
        r#" "comment":{ "#,
            r#" "user":{ "#,
                r#" "login":"testu", "#,
                r#" "type":"User" "#,
            r#" }, "#,
            r#" "body":"@AelitaBot r+" "#,
        r#" }, "#,
        r#" "repository":{ "#,
            r#" "name":"testp", "#,
            r#" "owner":{ "#,
                r#" "login":"AelitaBot", "#,
                r#" "type":"User" "#,
            r#" } "#,
        r#" } "#,
    r#" } "#);
    http_headers.set_raw("X-Hub-Signature", vec![
        format!("sha1={}", openssl::crypto::hmac::hmac(
            openssl::crypto::hash::Type::SHA1,
            "ME_SECRET_LOL".as_bytes(),
            body.as_bytes(),
        ).to_hex()).into_bytes()
    ]);
    http_client.post("http://localhost:9001")
        .body(body)
        .headers(http_headers)
        .send()
        .unwrap();

    info!("Aelita asks if we're an organization. We're not, for this test.");
    single_request(&mut github_server, |req, mut res| {
        assert_eq!(
            req.uri,
            RequestUri::AbsolutePath("/repos/AelitaBot/testp".to_owned())
        );
        assert_eq!(
            &req.headers.get_raw("Authorization").unwrap()[0][..],
            b"token MY_PERSONAL_ACCESS_TOKEN"
        );
        *res.status_mut() = StatusCode::Ok;
        res.send(concat!(r#" { "#,
            r#" "name":"testp", "#,
            r#" "owner":{"login":"AelitaBot","type":"User"} "#,
            r#" } "#).as_bytes()).unwrap();
    });

    info!("Aelita checks if user has permission to do that.");
    single_request(&mut github_server, |req, mut res| {
        let path = "/repos/AelitaBot/testp/collaborators/testu".to_owned();
        assert_eq!(
            req.uri,
            RequestUri::AbsolutePath(path)
        );
        assert_eq!(
            &req.headers.get_raw("Authorization").unwrap()[0][..],
            b"token MY_PERSONAL_ACCESS_TOKEN"
        );
        *res.status_mut() = StatusCode::NoContent;
        res.send(&[]).unwrap();
    });

    info!("Wait a sec for it to finish merging.");
    thread::sleep(time::Duration::new(4, 0));

    info!("Aelita does the merge.");
    let mut commit_string = String::new();
    File::open(Path::new("tests/cache/origin/.git/refs/heads/staging"))
        .unwrap()
        .read_to_string(&mut commit_string)
        .unwrap();
    commit_string = commit_string.replace("\n", "").replace("\r", "");

    info!("Github sends start notification to Aelita.");
    let http_client = Client::new();
    let mut http_headers = Headers::new();
    http_headers.set_raw("X-Github-Event", vec![b"status".to_vec()]);
    let body = concat!(r#" { "#,
            r#" "state": "pending", "#,
            r#" "target_url": "http://example.com/target_url", "#,
            r#" "context": "ci/test", "#,
            r#" "sha": "CMMT", "#,
            r#" "repository": { "#,
                r#" "name": "testp", "#,
                r#" "owner": {"login":"AelitaBot"} "#,
            r#" } "#,
        r#" } "#
        ).replace("CMMT", &commit_string);
    http_headers.set_raw("X-Hub-Signature", vec![
        format!("sha1={}", openssl::crypto::hmac::hmac(
            openssl::crypto::hash::Type::SHA1,
            "ME_SECRET_LOL".as_bytes(),
            body.as_bytes(),
        ).to_hex()).into_bytes()
    ]);
    http_client.post("http://localhost:9002")
        .body(body.as_bytes())
        .headers(http_headers)
        .send()
        .unwrap();

    info!("Github sends finished notification to Aelita.");
    let http_client = Client::new();
    let mut http_headers = Headers::new();
    http_headers.set_raw("X-Github-Event", vec![b"status".to_vec()]);
    let body = concat!(r#" { "#,
            r#" "state": "success", "#,
            r#" "target_url": "http://example.com/target_url", "#,
            r#" "context": "ci/test", "#,
            r#" "sha": "CMMT", "#,
            r#" "repository": { "#,
                r#" "name": "testp", "#,
                r#" "owner": {"login":"AelitaBot"} "#,
            r#" } "#,
        r#" } "#
        ).replace("CMMT", &commit_string);
    http_headers.set_raw("X-Hub-Signature", vec![
        format!("sha1={}", openssl::crypto::hmac::hmac(
            openssl::crypto::hash::Type::SHA1,
            "ME_SECRET_LOL".as_bytes(),
            body.as_bytes(),
        ).to_hex()).into_bytes()
    ]);
    http_client.post("http://localhost:9002")
        .body(body.as_bytes())
        .headers(http_headers)
        .send()
        .unwrap();

    info!("Wait a sec for it to finish pushing.");
    thread::sleep(time::Duration::new(2, 0));

    info!("Aelita fast-forwards master.");
    let mut master_string = String::new();
    File::open(Path::new("tests/cache/origin/.git/refs/heads/master"))
        .unwrap()
        .read_to_string(&mut master_string)
        .unwrap();
    master_string = master_string.replace("\n", "").replace("\r", "");
    assert!(master_string != "e16d1eca074ae29ac1812e14316e96f3117d0675");

    aelita.kill().unwrap();
}

#[test]
fn one_item_github_round_trip_cloud() {
    let _lock = ONE_AT_A_TIME.lock();
    START.call_once(|| env_logger::init().unwrap());

    if !Path::new(EXECUTABLE).exists() {
        panic!("Integration tests require the executable to be built.");
    }

    Command::new("/bin/rm")
        .current_dir("./tests/")
        .arg("db.sqlite")
        .output()
        .unwrap();

    let mut github_server = HttpListener::new(&"localhost:9011").unwrap();
    let mut jenkins_server = HttpListener::new(&"localhost:9012").unwrap();
    let mut github_git_server = HttpListener::new(&"localhost:9013").unwrap();

    let executable = Path::new(EXECUTABLE).canonicalize().unwrap();
    let mut aelita = Command::new(executable)
        .current_dir("./tests/")
        .arg("test-github-round-trip-cloud.toml")
        .spawn()
        .unwrap();

    info!("Wait a sec for it to finish starting.");
    thread::sleep(time::Duration::new(2, 0));

    info!("Pull request comes into existance.");
    let http_client = Client::new();
    let mut http_headers = Headers::new();
    http_headers.set_raw("X-Github-Event", vec![b"pull_request".to_vec()]);
    let body = concat!(r#" { "#,
        r#" "action":"opened", "#,
        r#" "repository":{ "#,
            r#" "name":"testp", "#,
            r#" "owner":{"login":"AelitaBot","type":"User"} "#,
        r#" }, "#,
        r#" "pull_request":{ "#,
            r#" "title":"HA!", "#,
            r#" "html_url":"http://github.com/testu/testp/pull_request/1", "#,
            r#" "state":"opened", "#,
            r#" "number":1, "#,
            r#" "head":{ "#,
                r#" "sha":"55016813274e906e4cbfed97be83e19e6cd93d91" "#,
            r#" } "#,
        r#" } "#,
    r#" } "#);
    http_headers.set_raw("X-Hub-Signature", vec![
        format!("sha1={}", openssl::crypto::hmac::hmac(
            openssl::crypto::hash::Type::SHA1,
            "ME_SECRET_LOL".as_bytes(),
            body.as_bytes(),
        ).to_hex()).into_bytes()
    ]);
    http_client.post("http://localhost:9001")
        .body(body.as_bytes())
        .headers(http_headers)
        .send()
        .unwrap();

    info!("User posts comment to mark pull request reviewed.");
    let http_client = Client::new();
    let mut http_headers = Headers::new();
    http_headers.set_raw("X-Github-Event", vec![b"issue_comment".to_vec()]);
    let body = concat!(r#" { "#,
        r#" "issue":{ "#,
            r#" "number":1, "#,
            r#" "title":"My PR!", "#,
            r#" "body":"Test", "#,
            r#" "pull_request":{ "#,
                r#" "html_url":"http://github.com/testu/testp/pull_request/1" "#,
            r#" }, "#,
            r#" "state":"opened", "#,
            r#" "user":{ "#,
                r#" "login":"testu", "#,
                r#" "type":"User" "#,
            r#" } "#,
        r#" }, "#,
        r#" "comment":{ "#,
            r#" "user":{ "#,
                r#" "login":"testu", "#,
                r#" "type":"User" "#,
            r#" }, "#,
            r#" "body":"@AelitaBot r+" "#,
        r#" }, "#,
        r#" "repository":{ "#,
            r#" "name":"testp", "#,
            r#" "owner":{ "#,
                r#" "login":"AelitaBot", "#,
                r#" "type":"User" "#,
            r#" } "#,
        r#" } "#,
    r#" } "#);
    http_headers.set_raw("X-Hub-Signature", vec![
        format!("sha1={}", openssl::crypto::hmac::hmac(
            openssl::crypto::hash::Type::SHA1,
            "ME_SECRET_LOL".as_bytes(),
            body.as_bytes(),
        ).to_hex()).into_bytes()
    ]);
    http_client.post("http://localhost:9001")
        .body(body)
        .headers(http_headers)
        .send()
        .unwrap();

    info!("Aelita asks if we're an organization. We're not, for this test.");
    single_request(&mut github_server, |req, mut res| {
        assert_eq!(
            req.uri,
            RequestUri::AbsolutePath("/repos/AelitaBot/testp".to_owned())
        );
        assert_eq!(
            &req.headers.get_raw("Authorization").unwrap()[0][..],
            b"token MY_PERSONAL_ACCESS_TOKEN"
        );
        *res.status_mut() = StatusCode::Ok;
        res.send(concat!(r#" { "#,
            r#" "name":"testp", "#,
            r#" "owner":{"login":"AelitaBot","type":"User"} "#,
            r#" } "#).as_bytes()).unwrap();
    });

    info!("Aelita checks if user has permission to do that.");
    single_request(&mut github_server, |req, mut res| {
        let path = "/repos/AelitaBot/testp/collaborators/testu".to_owned();
        assert_eq!(
            req.uri,
            RequestUri::AbsolutePath(path)
        );
        assert_eq!(
            &req.headers.get_raw("Authorization").unwrap()[0][..],
            b"token MY_PERSONAL_ACCESS_TOKEN"
        );
        *res.status_mut() = StatusCode::NoContent;
        res.send(&[]).unwrap();
    });

    info!("Aelita checks for the current contents of master.");
    single_request(&mut github_git_server, |req, mut res| {
        let path = "/repos/AelitaBot/testp/git/refs/heads/master".to_owned();
        assert_eq!(
            req.uri,
            RequestUri::AbsolutePath(path)
        );
        assert_eq!(
            &req.headers.get_raw("Authorization").unwrap()[0][..],
            b"token MY_PERSONAL_ACCESS_TOKEN"
        );
        *res.status_mut() = StatusCode::Ok;
        res.send(concat!(
            r#"{"object":"#,
                r#"{"sha":"aa218f56b14c9653891f9e74264a383fa43fefbd"}"#,
            r#"}"#
        ).as_bytes()).unwrap();
    });

    info!("Aelita checks for the current contents of staging. It matches.");
    single_request(&mut github_git_server, |req, mut res| {
        let path = "/repos/AelitaBot/testp/git/refs/heads/staging".to_owned();
        assert_eq!(
            req.uri,
            RequestUri::AbsolutePath(path)
        );
        assert_eq!(
            &req.headers.get_raw("Authorization").unwrap()[0][..],
            b"token MY_PERSONAL_ACCESS_TOKEN"
        );
        *res.status_mut() = StatusCode::Ok;
        res.send(concat!(
            r#"{"object":"#,
                r#"{"sha":"aa218f56b14c9653891f9e74264a383fa43fefbd"}"#,
            r#"}"#
        ).as_bytes()).unwrap();
    });

    info!("Aelita merges staging to master.");
    single_request(&mut github_git_server, |req, mut res| {
        let path = "/repos/AelitaBot/testp/merges".to_owned();
        assert_eq!(
            req.uri,
            RequestUri::AbsolutePath(path)
        );
        assert_eq!(
            &req.headers.get_raw("Authorization").unwrap()[0][..],
            b"token MY_PERSONAL_ACCESS_TOKEN"
        );
        *res.status_mut() = StatusCode::Ok;
        res.send(concat!(
            r#"{"sha":"#,
                r#""ba218f56b14c9653891f9e74264a383fa43fefbd""#,
            r#"}"#
        ).as_bytes()).unwrap();
    });

    info!("Aelita sends build trigger.");
    single_request(&mut jenkins_server, |req, mut res| {
        let path = "/job/testp/build?token=MY_BUILD_TOKEN".to_owned();
        assert_eq!(
            req.uri,
            RequestUri::AbsolutePath(path)
        );
        assert_eq!(
            req.headers.get::<Authorization<Basic>>().unwrap().0,
            Basic{
                username: "AelitaBot".to_owned(),
                password: Some("MY_JENKINS_API_TOKEN".to_owned()),
            }
        );
        *res.status_mut() = StatusCode::NoContent;
        res.send(&[]).unwrap();
    });

    info!("Jenkins sends start notification to Aelita.");
    let mut tcp_client = TcpStream::connect("localhost:9002").unwrap();
    tcp_client.write(
        br#"{"name":"testp","build":{"phase":"STARTED","full_url":"http://jenkins.com/job/1/","scm":{"commit":"ba218f56b14c9653891f9e74264a383fa43fefbd"}}}"#
    ).unwrap();
    drop(tcp_client);

    info!("Jenkins sends finished notification to Aelita.");
    let mut tcp_client = TcpStream::connect("localhost:9002").unwrap();
    tcp_client.write(
        br#"{"name":"testp","build":{"phase":"COMPLETED","status":"SUCCESS","full_url":"http://jenkins.com/job/1/","scm":{"commit":"ba218f56b14c9653891f9e74264a383fa43fefbd"}}}"#
    ).unwrap();
    drop(tcp_client);

    info!("Aelita marks staging a success.");
    single_request(&mut github_git_server, |req, mut res| {
        let path = format!("/repos/AelitaBot/testp/statuses/{}",
            "ba218f56b14c9653891f9e74264a383fa43fefbd"
        );
        assert_eq!(
            req.uri,
            RequestUri::AbsolutePath(path)
        );
        assert_eq!(
            &req.headers.get_raw("Authorization").unwrap()[0][..],
            b"token MY_PERSONAL_ACCESS_TOKEN"
        );
        #[derive(Deserialize)]
        struct StatusDesc {
            state: String,
            target_url: Option<String>,
            description: String,
            context: String,
        }
        let desc: StatusDesc =
            serde_json::from_reader(req).expect("valid JSON");
        *res.status_mut() = StatusCode::Ok;
        res.send(&[]).unwrap();
        assert_eq!("success", &desc.state[..]);
    });

    info!("Aelita fast-forwards master to staging.");
    let master_string = single_request(&mut github_git_server, |req, mut res| {
        let path = "/repos/AelitaBot/testp/git/refs/heads/master".to_owned();
        assert_eq!(
            req.uri,
            RequestUri::AbsolutePath(path)
        );
        assert_eq!(
            &req.headers.get_raw("Authorization").unwrap()[0][..],
            b"token MY_PERSONAL_ACCESS_TOKEN"
        );
        #[derive(Deserialize)]
        struct FFDesc {
            force: bool,
            sha: String,
        }
        let desc: FFDesc = serde_json::from_reader(req).expect("valid JSON");
        *res.status_mut() = StatusCode::Ok;
        res.send(&[]).unwrap();
        assert!(!desc.force);
        desc.sha
    });
    assert_eq!("ba218f56b14c9653891f9e74264a383fa43fefbd", master_string);

    aelita.kill().unwrap();
}

#[test]
fn one_item_github_round_trip_cloud_no_staging() {
    let _lock = ONE_AT_A_TIME.lock();
    START.call_once(|| env_logger::init().unwrap());

    if !Path::new(EXECUTABLE).exists() {
        panic!("Integration tests require the executable to be built.");
    }

    Command::new("/bin/rm")
        .current_dir("./tests/")
        .arg("db.sqlite")
        .output()
        .unwrap();

    let mut github_server = HttpListener::new(&"localhost:9011").unwrap();
    let mut jenkins_server = HttpListener::new(&"localhost:9012").unwrap();
    let mut github_git_server = HttpListener::new(&"localhost:9013").unwrap();

    let executable = Path::new(EXECUTABLE).canonicalize().unwrap();
    let mut aelita = Command::new(executable)
        .current_dir("./tests/")
        .arg("test-github-round-trip-cloud.toml")
        .spawn()
        .unwrap();

    info!("Wait a sec for it to finish starting.");
    thread::sleep(time::Duration::new(2, 0));

    info!("Pull request comes into existance.");
    let http_client = Client::new();
    let mut http_headers = Headers::new();
    http_headers.set_raw("X-Github-Event", vec![b"pull_request".to_vec()]);
    let body = concat!(r#" { "#,
        r#" "action":"opened", "#,
        r#" "repository":{ "#,
            r#" "name":"testp", "#,
            r#" "owner":{"login":"AelitaBot","type":"User"} "#,
        r#" }, "#,
        r#" "pull_request":{ "#,
            r#" "title":"HA!", "#,
            r#" "html_url":"http://github.com/testu/testp/pull_request/1", "#,
            r#" "state":"opened", "#,
            r#" "number":1, "#,
            r#" "head":{ "#,
                r#" "sha":"55016813274e906e4cbfed97be83e19e6cd93d91" "#,
            r#" } "#,
        r#" } "#,
    r#" } "#);
    http_headers.set_raw("X-Hub-Signature", vec![
        format!("sha1={}", openssl::crypto::hmac::hmac(
            openssl::crypto::hash::Type::SHA1,
            "ME_SECRET_LOL".as_bytes(),
            body.as_bytes(),
        ).to_hex()).into_bytes()
    ]);
    http_client.post("http://localhost:9001")
        .body(body.as_bytes())
        .headers(http_headers)
        .send()
        .unwrap();

    info!("User posts comment to mark pull request reviewed.");
    let http_client = Client::new();
    let mut http_headers = Headers::new();
    http_headers.set_raw("X-Github-Event", vec![b"issue_comment".to_vec()]);
    let body = concat!(r#" { "#,
        r#" "issue":{ "#,
            r#" "number":1, "#,
            r#" "title":"My PR!", "#,
            r#" "body":"Test", "#,
            r#" "pull_request":{ "#,
                r#" "html_url":"http://github.com/testu/testp/pull_request/1" "#,
            r#" }, "#,
            r#" "state":"opened", "#,
            r#" "user":{ "#,
                r#" "login":"testu", "#,
                r#" "type":"User" "#,
            r#" } "#,
        r#" }, "#,
        r#" "comment":{ "#,
            r#" "user":{ "#,
                r#" "login":"testu", "#,
                r#" "type":"User" "#,
            r#" }, "#,
            r#" "body":"@AelitaBot r+" "#,
        r#" }, "#,
        r#" "repository":{ "#,
            r#" "name":"testp", "#,
            r#" "owner":{ "#,
                r#" "login":"AelitaBot", "#,
                r#" "type":"User" "#,
            r#" } "#,
        r#" } "#,
    r#" } "#);
    http_headers.set_raw("X-Hub-Signature", vec![
        format!("sha1={}", openssl::crypto::hmac::hmac(
            openssl::crypto::hash::Type::SHA1,
            "ME_SECRET_LOL".as_bytes(),
            body.as_bytes(),
        ).to_hex()).into_bytes()
    ]);
    http_client.post("http://localhost:9001")
        .body(body)
        .headers(http_headers)
        .send()
        .unwrap();

    info!("Aelita asks if we're an organization. We're not, for this test.");
    single_request(&mut github_server, |req, mut res| {
        assert_eq!(
            req.uri,
            RequestUri::AbsolutePath("/repos/AelitaBot/testp".to_owned())
        );
        assert_eq!(
            &req.headers.get_raw("Authorization").unwrap()[0][..],
            b"token MY_PERSONAL_ACCESS_TOKEN"
        );
        *res.status_mut() = StatusCode::Ok;
        res.send(concat!(r#" { "#,
            r#" "name":"testp", "#,
            r#" "owner":{"login":"AelitaBot","type":"User"} "#,
            r#" } "#).as_bytes()).unwrap();
    });

    info!("Aelita checks if user has permission to do that.");
    single_request(&mut github_server, |req, mut res| {
        let path = "/repos/AelitaBot/testp/collaborators/testu".to_owned();
        assert_eq!(
            req.uri,
            RequestUri::AbsolutePath(path)
        );
        assert_eq!(
            &req.headers.get_raw("Authorization").unwrap()[0][..],
            b"token MY_PERSONAL_ACCESS_TOKEN"
        );
        *res.status_mut() = StatusCode::NoContent;
        res.send(&[]).unwrap();
    });

    info!("Aelita checks for the current contents of master.");
    single_request(&mut github_git_server, |req, mut res| {
        let path = "/repos/AelitaBot/testp/git/refs/heads/master".to_owned();
        assert_eq!(
            req.uri,
            RequestUri::AbsolutePath(path)
        );
        assert_eq!(
            &req.headers.get_raw("Authorization").unwrap()[0][..],
            b"token MY_PERSONAL_ACCESS_TOKEN"
        );
        *res.status_mut() = StatusCode::Ok;
        res.send(concat!(
            r#"{"object":"#,
                r#"{"sha":"aa218f56b14c9653891f9e74264a383fa43fefbd"}"#,
            r#"}"#
        ).as_bytes()).unwrap();
    });

    info!("Aelita checks for the current contents of staging. There isn't.");
    single_request(&mut github_git_server, |req, mut res| {
        let path = "/repos/AelitaBot/testp/git/refs/heads/staging".to_owned();
        assert_eq!(
            req.uri,
            RequestUri::AbsolutePath(path)
        );
        assert_eq!(
            &req.headers.get_raw("Authorization").unwrap()[0][..],
            b"token MY_PERSONAL_ACCESS_TOKEN"
        );
        *res.status_mut() = StatusCode::NotFound;
        res.send(&[]).unwrap();
    });

    info!("Aelita sets staging to master.");
    single_request(&mut github_git_server, |req, mut res| {
        let path = "/repos/AelitaBot/testp/git/refs".to_owned();
        assert_eq!(
            req.uri,
            RequestUri::AbsolutePath(path)
        );
        assert_eq!(
            &req.headers.get_raw("Authorization").unwrap()[0][..],
            b"token MY_PERSONAL_ACCESS_TOKEN"
        );
        assert_eq!(
            req.method,
            Method::Post
        );
        *res.status_mut() = StatusCode::Ok;
        res.send(&[]).unwrap();
    });

    info!("Aelita merges staging to master.");
    single_request(&mut github_git_server, |req, mut res| {
        let path = "/repos/AelitaBot/testp/merges".to_owned();
        assert_eq!(
            req.uri,
            RequestUri::AbsolutePath(path)
        );
        assert_eq!(
            &req.headers.get_raw("Authorization").unwrap()[0][..],
            b"token MY_PERSONAL_ACCESS_TOKEN"
        );
        *res.status_mut() = StatusCode::Ok;
        res.send(concat!(
            r#"{"sha":"#,
                r#""ba218f56b14c9653891f9e74264a383fa43fefbd""#,
            r#"}"#
        ).as_bytes()).unwrap();
    });

    info!("Aelita sends build trigger.");
    single_request(&mut jenkins_server, |req, mut res| {
        let path = "/job/testp/build?token=MY_BUILD_TOKEN".to_owned();
        assert_eq!(
            req.uri,
            RequestUri::AbsolutePath(path)
        );
        assert_eq!(
            req.headers.get::<Authorization<Basic>>().unwrap().0,
            Basic{
                username: "AelitaBot".to_owned(),
                password: Some("MY_JENKINS_API_TOKEN".to_owned()),
            }
        );
        *res.status_mut() = StatusCode::NoContent;
        res.send(&[]).unwrap();
    });

    info!("Jenkins sends start notification to Aelita.");
    let mut tcp_client = TcpStream::connect("localhost:9002").unwrap();
    tcp_client.write(
        br#"{"name":"testp","build":{"phase":"STARTED","full_url":"http://jenkins.com/job/1/","scm":{"commit":"ba218f56b14c9653891f9e74264a383fa43fefbd"}}}"#
    ).unwrap();
    drop(tcp_client);

    info!("Jenkins sends finished notification to Aelita.");
    let mut tcp_client = TcpStream::connect("localhost:9002").unwrap();
    tcp_client.write(
        br#"{"name":"testp","build":{"phase":"COMPLETED","status":"SUCCESS","full_url":"http://jenkins.com/job/1/","scm":{"commit":"ba218f56b14c9653891f9e74264a383fa43fefbd"}}}"#
    ).unwrap();
    drop(tcp_client);

    info!("Aelita marks staging a success.");
    single_request(&mut github_git_server, |req, mut res| {
        let path = format!("/repos/AelitaBot/testp/statuses/{}",
            "ba218f56b14c9653891f9e74264a383fa43fefbd"
        );
        assert_eq!(
            req.uri,
            RequestUri::AbsolutePath(path)
        );
        assert_eq!(
            &req.headers.get_raw("Authorization").unwrap()[0][..],
            b"token MY_PERSONAL_ACCESS_TOKEN"
        );
        #[derive(Deserialize)]
        struct StatusDesc {
            state: String,
            target_url: Option<String>,
            description: String,
            context: String,
        }
        let desc: StatusDesc =
            serde_json::from_reader(req).expect("valid JSON");
        *res.status_mut() = StatusCode::Ok;
        res.send(&[]).unwrap();
        assert_eq!("success", &desc.state[..]);
    });

    info!("Aelita fast-forwards master to staging.");
    let master_string = single_request(&mut github_git_server, |req, mut res| {
        let path = "/repos/AelitaBot/testp/git/refs/heads/master".to_owned();
        assert_eq!(
            req.uri,
            RequestUri::AbsolutePath(path)
        );
        assert_eq!(
            &req.headers.get_raw("Authorization").unwrap()[0][..],
            b"token MY_PERSONAL_ACCESS_TOKEN"
        );
        #[derive(Deserialize)]
        struct FFDesc {
            force: bool,
            sha: String,
        }
        let desc: FFDesc = serde_json::from_reader(req).expect("valid JSON");
        *res.status_mut() = StatusCode::Ok;
        res.send(&[]).unwrap();
        assert!(!desc.force);
        desc.sha
    });
    assert_eq!("ba218f56b14c9653891f9e74264a383fa43fefbd", master_string);

    aelita.kill().unwrap();
}

#[test]
fn one_item_team_github_round_trip_with_postgres() {
    let _lock = ONE_AT_A_TIME.lock();
    START.call_once(|| env_logger::init().unwrap());

    if !Path::new(EXECUTABLE).exists() {
        panic!("Integration tests require the executable to be built.");
    }

    if !Path::new("/usr/bin/docker").exists() {
        warn!("Test skipped because no docker.");
        return;
    }

    let mut github_server = HttpListener::new(&"localhost:9011").unwrap();
    let mut jenkins_server = HttpListener::new(&"localhost:9012").unwrap();

    Command::new("/bin/tar")
        .current_dir("./tests/")
        .arg("-xvf")
        .arg("cache.tar.gz")
        .output()
        .unwrap();

    Command::new("/bin/rm")
        .current_dir("./tests/")
        .arg("db.sqlite")
        .output()
        .unwrap();

    Command::new("/usr/bin/docker")
        .current_dir("./tests/")
        .arg("rm")
        .arg("-f")
        .arg("aelita-test-github-round-trip-postgres")
        .output()
        .unwrap();
    Command::new("/usr/bin/docker")
        .current_dir("./tests/")
        .arg("run")
        .arg("--name")
        .arg("aelita-test-github-round-trip-postgres")
        .arg("-p")
        .arg("5432:5432")
        .arg("-e")
        .arg("POSTGRES_USER=postgres")
        .arg("-e")
        .arg("POSTGRES_PASSWORD=")
        .arg("-d")
        .arg("postgres")
        .output()
        .unwrap();

    info!("Wait a sec for Postgres to start.");
    thread::sleep(time::Duration::new(5, 0));

    let executable = Path::new(EXECUTABLE).canonicalize().unwrap();
    let mut aelita = Command::new(executable.clone())
        .current_dir("./tests/")
        .arg("test-github-round-trip-with-postgres.toml")
        .spawn()
        .unwrap();

    info!("Wait a sec for it to finish writing.");
    thread::sleep(time::Duration::new(2, 0));

    info!("Make sure it didn't create a db.sqlite");
    assert!(!Path::new("./tests/db.sqlite").exists());

    info!("Restart Aelita, forcing it to use the persistent team cache.");
    aelita.kill().unwrap();
    thread::sleep(time::Duration::new(1, 0));
    let mut aelita = Command::new(executable)
        .current_dir("./tests/")
        .arg("test-github-round-trip-with-postgres.toml")
        .spawn()
        .unwrap();

    info!("Wait a sec for it to finish starting.");
    thread::sleep(time::Duration::new(2, 0));

    info!("Pull request comes into existance.");
    let http_client = Client::new();
    let mut http_headers = Headers::new();
    http_headers.set_raw("X-Github-Event", vec![b"pull_request".to_vec()]);
    let body = concat!(r#" { "#,
        r#" "action":"opened", "#,
        r#" "repository":{ "#,
            r#" "name":"testp", "#,
            r#" "owner":{"login":"AelitaBot","type":"User"} "#,
        r#" }, "#,
        r#" "pull_request":{ "#,
            r#" "title":"HA!", "#,
            r#" "html_url":"http://github.com/testu/testp/pull_request/1", "#,
            r#" "state":"opened", "#,
            r#" "number":1, "#,
            r#" "head":{ "#,
                r#" "sha":"55016813274e906e4cbfed97be83e19e6cd93d91" "#,
            r#" } "#,
        r#" } "#,
    r#" } "#);
    http_headers.set_raw("X-Hub-Signature", vec![
        format!("sha1={}", openssl::crypto::hmac::hmac(
            openssl::crypto::hash::Type::SHA1,
            "ME_SECRET_LOL".as_bytes(),
            body.as_bytes(),
        ).to_hex()).into_bytes()
    ]);
    http_client.post("http://localhost:9001")
        .body(body.as_bytes())
        .headers(http_headers)
        .send()
        .unwrap();

    info!("User posts comment to mark pull request reviewed.");
    let http_client = Client::new();
    let mut http_headers = Headers::new();
    http_headers.set_raw("X-Github-Event", vec![b"issue_comment".to_vec()]);
    let body = concat!(r#" { "#,
        r#" "issue":{ "#,
            r#" "number":1, "#,
            r#" "title":"My PR!", "#,
            r#" "body":"Test", "#,
            r#" "pull_request":{ "#,
                r#" "html_url":"http://github.com/testu/testp/pull_request/1" "#,
            r#" }, "#,
            r#" "state":"opened", "#,
            r#" "user":{ "#,
                r#" "login":"testu", "#,
                r#" "type":"User" "#,
            r#" } "#,
        r#" }, "#,
        r#" "comment":{ "#,
            r#" "user":{ "#,
                r#" "login":"testu", "#,
                r#" "type":"User" "#,
            r#" }, "#,
            r#" "body":"@AelitaBot r+" "#,
        r#" }, "#,
        r#" "repository":{ "#,
            r#" "name":"testp", "#,
            r#" "owner":{ "#,
                r#" "login":"AelitaBot", "#,
                r#" "type":"User" "#,
            r#" } "#,
        r#" } "#,
    r#" } "#);
    http_headers.set_raw("X-Hub-Signature", vec![
        format!("sha1={}", openssl::crypto::hmac::hmac(
            openssl::crypto::hash::Type::SHA1,
            "ME_SECRET_LOL".as_bytes(),
            body.as_bytes(),
        ).to_hex()).into_bytes()
    ]);
    http_client.post("http://localhost:9001")
        .body(body)
        .headers(http_headers)
        .send()
        .unwrap();

    info!("Aelita asks if we're an organization. We are, for this test.");
    single_request(&mut github_server, |req, mut res| {
        assert_eq!(
            req.uri,
            RequestUri::AbsolutePath("/repos/AelitaBot/testp".to_owned())
        );
        assert_eq!(
            &req.headers.get_raw("Authorization").unwrap()[0][..],
            b"token MY_PERSONAL_ACCESS_TOKEN"
        );
        *res.status_mut() = StatusCode::Ok;
        res.send(concat!(r#" { "#,
            r#" "name":"testp", "#,
            r#" "owner":{"login":"AelitaBot","type":"Organization"} "#,
            r#" } "#).as_bytes()).unwrap();
    });

    info!("Aelita gets a list of teams.");
    single_request(&mut github_server, |req, mut res| {
        assert_eq!(
            req.uri,
            RequestUri::AbsolutePath("/orgs/AelitaBot/teams".to_owned())
        );
        assert_eq!(
            &req.headers.get_raw("Authorization").unwrap()[0][..],
            b"token MY_PERSONAL_ACCESS_TOKEN"
        );
        *res.status_mut() = StatusCode::Ok;
        res.send(concat!(r#" [ "#,
            r#" { "id":1, "slug":"Potato" } "#,
            r#" ] "#).as_bytes()).unwrap();
    });

    info!("Aelita checks if Potato has write permission. It does.");
    single_request(&mut github_server, |req, mut res| {
        assert_eq!(
            req.uri,
            RequestUri::AbsolutePath(
                "/teams/1/repos/AelitaBot/testp".to_owned()
            )
        );
        assert_eq!(
            &req.headers.get_raw("Authorization").unwrap()[0][..],
            b"token MY_PERSONAL_ACCESS_TOKEN"
        );
        *res.status_mut() = StatusCode::Ok;
        res.send(concat!(r#" { "#,
            r#" "permissions": { "pull":true,"push":true,"admin":false } "#,
            r#" } "#).as_bytes()).unwrap();
    });

    info!("Aelita checks if user is member of Potato.");
    single_request(&mut github_server, |req, mut res| {
        let path = "/teams/1/members/testu".to_owned();
        assert_eq!(
            req.uri,
            RequestUri::AbsolutePath(path)
        );
        assert_eq!(
            &req.headers.get_raw("Authorization").unwrap()[0][..],
            b"token MY_PERSONAL_ACCESS_TOKEN"
        );
        *res.status_mut() = StatusCode::NoContent;
        res.send(&[]).unwrap();
    });

    info!("Aelita sends build trigger.");
    single_request(&mut jenkins_server, |req, mut res| {
        let path = "/job/testp/build?token=MY_BUILD_TOKEN".to_owned();
        assert_eq!(
            req.uri,
            RequestUri::AbsolutePath(path)
        );
        assert_eq!(
            req.headers.get::<Authorization<Basic>>().unwrap().0,
            Basic{
                username: "AelitaBot".to_owned(),
                password: Some("MY_JENKINS_API_TOKEN".to_owned()),
            }
        );
        *res.status_mut() = StatusCode::NoContent;
        res.send(&[]).unwrap();
    });

    info!("Aelita does the merge.");
    let mut commit_string = String::new();
    File::open(Path::new("tests/cache/origin/.git/refs/heads/staging"))
        .unwrap()
        .read_to_string(&mut commit_string)
        .unwrap();
    commit_string = commit_string.replace("\n", "").replace("\r", "");

    info!("Jenkins sends start notification to Aelita.");
    let mut tcp_client = TcpStream::connect("localhost:9002").unwrap();
    tcp_client.write(
        r#"{"name":"testp","build":{"phase":"STARTED","full_url":"http://jenkins.com/job/1/","scm":{"commit":"CMMT"}}}"#
            .replace("CMMT", &commit_string)
            .as_bytes()
    ).unwrap();
    drop(tcp_client);

    info!("Jenkins sends finished notification to Aelita.");
    let mut tcp_client = TcpStream::connect("localhost:9002").unwrap();
    tcp_client.write(
        r#"{"name":"testp","build":{"phase":"COMPLETED","status":"SUCCESS","full_url":"http://jenkins.com/job/1/","scm":{"commit":"CMMT"}}}"#
            .replace("CMMT", &commit_string)
            .as_bytes()
    ).unwrap();
    drop(tcp_client);

    info!("Wait a sec for it to finish pushing.");
    thread::sleep(time::Duration::new(2, 0));

    info!("Aelita fast-forwards master.");
    let mut master_string = String::new();
    File::open(Path::new("tests/cache/origin/.git/refs/heads/master"))
        .unwrap()
        .read_to_string(&mut master_string)
        .unwrap();
    master_string = master_string.replace("\n", "").replace("\r", "");
    assert!(master_string != "e16d1eca074ae29ac1812e14316e96f3117d0675");

    aelita.kill().unwrap();

    Command::new("/usr/bin/docker")
        .current_dir("./tests/")
        .arg("rm")
        .arg("-f")
        .arg("aelita-test-github-round-trip-postgres")
        .output()
        .unwrap();

    info!("Make sure it didn't create a db.sqlite");
    assert!(!Path::new("./tests/db.sqlite").exists());
}

#[test]
fn one_item_github_round_trip_cloud_with_postgres_12f() {
    let _lock = ONE_AT_A_TIME.lock();
    START.call_once(|| env_logger::init().unwrap());

    if !Path::new(EXECUTABLE).exists() {
        panic!("Integration tests require the executable to be built.");
    }

    if !Path::new("/usr/bin/docker").exists() {
        warn!("Test skipped because no docker.");
        return;
    }

    Command::new("/bin/rm")
        .current_dir("./tests/")
        .arg("db.sqlite")
        .output()
        .unwrap();

    Command::new("/usr/bin/docker")
        .current_dir("./tests/")
        .arg("rm")
        .arg("-f")
        .arg("aelita-test-github-round-trip-postgres")
        .output()
        .unwrap();
    Command::new("/usr/bin/docker")
        .current_dir("./tests/")
        .arg("run")
        .arg("--name")
        .arg("aelita-test-github-round-trip-postgres")
        .arg("-p")
        .arg("5432:5432")
        .arg("-e")
        .arg("POSTGRES_USER=postgres")
        .arg("-e")
        .arg("POSTGRES_PASSWORD=")
        .arg("-d")
        .arg("postgres")
        .output()
        .unwrap();

    info!("Wait a sec for it to finish starting.");
    thread::sleep(time::Duration::new(5, 0));

    let mut github_server = HttpListener::new(&"localhost:9011").unwrap();
    let mut github_git_server = HttpListener::new(&"localhost:9013").unwrap();

    let executable = Path::new(EXECUTABLE).canonicalize().unwrap();
    let mut aelita = Command::new(executable)
        .current_dir("./tests/")
        .arg("-12")
        .env("AELITA_UI_TYPE",
             "github")
        .env("AELITA_PIPELINE_DB",
             "postgresql://postgres@localhost/postgres")
        .env("AELITA_PROJECT_DB",
             "postgresql://postgres@localhost/postgres")
        .env("AELITA_UI_GITHUB_DB",
             "postgresql://postgres@localhost/postgres")
        .env("AELITA_UI_GITHUB_LISTEN",
             "localhost:9001")
        .env("AELITA_UI_GITHUB_HOST",
             "http://localhost:9011")
        .env("AELITA_UI_GITHUB_USER",
             "AelitaBot")
        .env("AELITA_UI_GITHUB_OWNER",
             "AelitaBot")
        .env("AELITA_UI_GITHUB_TOKEN",
             "MY_PERSONAL_ACCESS_TOKEN")
        .env("AELITA_UI_GITHUB_SECRET",
             "ME_SECRET_LOL")
        .env("AELITA_CI_TYPE",
             "github_status")
        .env("AELITA_CI_GITHUB_LISTEN",
             "localhost:9002")
        .env("AELITA_CI_GITHUB_SECRET",
             "ME_SECRET_LOL")
        .env("AELITA_VCS_TYPE",
             "github")
        .env("AELITA_VCS_GITHUB_HOST",
             "http://localhost:9013")
        .env("AELITA_VCS_GITHUB_TOKEN",
             "MY_PERSONAL_ACCESS_TOKEN")
        .env("AELITA_VIEW_LISTEN",
             "localhost:9014")
        .env("AELITA_VIEW_SECRET",
             "your mom")
        .spawn()
        .unwrap();

    info!("Wait a sec for it to finish starting.");
    thread::sleep(time::Duration::new(2, 0));

    info!("Add the pipeline to the database.");
    {
        let conn = postgres::Connection::connect(
            "postgresql://postgres@localhost/postgres",
            postgres::TlsMode::None,
        ).unwrap();
        conn.batch_execute(r###"
            INSERT INTO twelvef_config_pipeline
                (pipeline_id, name)
            VALUES
                (1, 'Test');
            INSERT INTO twelvef_github_projects
                (pipeline_id, try_pipeline_id, owner, repo)
            VALUES
                (1, NULL, 'AelitaBot', 'testp');
            INSERT INTO twelvef_github_status_pipelines
                (ci_id, owner, repo, context)
            VALUES
                (2, 'AelitaBot', 'testp', 'ci/test');
            INSERT INTO twelvef_config_pipeline_ci
                (ci_id, pipeline_id)
            VALUES
                (2, 1);
            INSERT INTO twelvef_github_git_pipelines
                (
                    pipeline_id,
                    owner,
                    repo,
                    master_branch,
                    staging_branch,
                    push_to_master
                )
            VALUES
                (1, 'AelitaBot', 'testp', 'master', 'staging', TRUE);
        "###).unwrap();
    }

    info!("Pull request comes into existance.");
    let http_client = Client::new();
    let mut http_headers = Headers::new();
    http_headers.set_raw("X-Github-Event", vec![b"pull_request".to_vec()]);
    let body = concat!(r#" { "#,
        r#" "action":"opened", "#,
        r#" "repository":{ "#,
            r#" "name":"testp", "#,
            r#" "owner":{"login":"AelitaBot","type":"User"} "#,
        r#" }, "#,
        r#" "pull_request":{ "#,
            r#" "title":"HA!", "#,
            r#" "html_url":"http://github.com/testu/testp/pull_request/1", "#,
            r#" "state":"opened", "#,
            r#" "number":1, "#,
            r#" "head":{ "#,
                r#" "sha":"55016813274e906e4cbfed97be83e19e6cd93d91" "#,
            r#" } "#,
        r#" } "#,
    r#" } "#);
    http_headers.set_raw("X-Hub-Signature", vec![
        format!("sha1={}", openssl::crypto::hmac::hmac(
            openssl::crypto::hash::Type::SHA1,
            "ME_SECRET_LOL".as_bytes(),
            body.as_bytes(),
        ).to_hex()).into_bytes()
    ]);
    http_client.post("http://localhost:9001")
        .body(body.as_bytes())
        .headers(http_headers)
        .send()
        .unwrap();

    info!("User posts comment to mark pull request reviewed.");
    let http_client = Client::new();
    let mut http_headers = Headers::new();
    http_headers.set_raw("X-Github-Event", vec![b"issue_comment".to_vec()]);
    let body = concat!(r#" { "#,
        r#" "issue":{ "#,
            r#" "number":1, "#,
            r#" "title":"My PR!", "#,
            r#" "body":"Test", "#,
            r#" "pull_request":{ "#,
                r#" "html_url":"http://github.com/testu/testp/pull_request/1" "#,
            r#" }, "#,
            r#" "state":"opened", "#,
            r#" "user":{ "#,
                r#" "login":"testu", "#,
                r#" "type":"User" "#,
            r#" } "#,
        r#" }, "#,
        r#" "comment":{ "#,
            r#" "user":{ "#,
                r#" "login":"testu", "#,
                r#" "type":"User" "#,
            r#" }, "#,
            r#" "body":"@AelitaBot r+" "#,
        r#" }, "#,
        r#" "repository":{ "#,
            r#" "name":"testp", "#,
            r#" "owner":{ "#,
                r#" "login":"AelitaBot", "#,
                r#" "type":"User" "#,
            r#" } "#,
        r#" } "#,
    r#" } "#);
    http_headers.set_raw("X-Hub-Signature", vec![
        format!("sha1={}", openssl::crypto::hmac::hmac(
            openssl::crypto::hash::Type::SHA1,
            "ME_SECRET_LOL".as_bytes(),
            body.as_bytes(),
        ).to_hex()).into_bytes()
    ]);
    http_client.post("http://localhost:9001")
        .body(body)
        .headers(http_headers)
        .send()
        .unwrap();

    info!("Aelita asks if we're an organization. We're not, for this test.");
    single_request(&mut github_server, |req, mut res| {
        assert_eq!(
            req.uri,
            RequestUri::AbsolutePath("/repos/AelitaBot/testp".to_owned())
        );
        assert_eq!(
            &req.headers.get_raw("Authorization").unwrap()[0][..],
            b"token MY_PERSONAL_ACCESS_TOKEN"
        );
        *res.status_mut() = StatusCode::Ok;
        res.send(concat!(r#" { "#,
            r#" "name":"testp", "#,
            r#" "owner":{"login":"AelitaBot","type":"User"} "#,
            r#" } "#).as_bytes()).unwrap();
    });

    info!("Aelita checks if user has permission to do that.");
    single_request(&mut github_server, |req, mut res| {
        let path = "/repos/AelitaBot/testp/collaborators/testu".to_owned();
        assert_eq!(
            req.uri,
            RequestUri::AbsolutePath(path)
        );
        assert_eq!(
            &req.headers.get_raw("Authorization").unwrap()[0][..],
            b"token MY_PERSONAL_ACCESS_TOKEN"
        );
        *res.status_mut() = StatusCode::NoContent;
        res.send(&[]).unwrap();
    });

    info!("Aelita checks for the current contents of master.");
    single_request(&mut github_git_server, |req, mut res| {
        let path = "/repos/AelitaBot/testp/git/refs/heads/master".to_owned();
        assert_eq!(
            req.uri,
            RequestUri::AbsolutePath(path)
        );
        assert_eq!(
            &req.headers.get_raw("Authorization").unwrap()[0][..],
            b"token MY_PERSONAL_ACCESS_TOKEN"
        );
        *res.status_mut() = StatusCode::Ok;
        res.send(concat!(
            r#"{"object":"#,
                r#"{"sha":"aa218f56b14c9653891f9e74264a383fa43fefbd"}"#,
            r#"}"#
        ).as_bytes()).unwrap();
    });

    info!("Aelita checks for the current contents of staging. Don't match.");
    single_request(&mut github_git_server, |req, mut res| {
        let path = "/repos/AelitaBot/testp/git/refs/heads/staging".to_owned();
        assert_eq!(
            req.uri,
            RequestUri::AbsolutePath(path)
        );
        assert_eq!(
            &req.headers.get_raw("Authorization").unwrap()[0][..],
            b"token MY_PERSONAL_ACCESS_TOKEN"
        );
        *res.status_mut() = StatusCode::Ok;
        res.send(concat!(
            r#"{"object":"#,
                r#"{"sha":"aa218f56b14c9653891f9e74264a383fa43fefbe"}"#,
            r#"}"#
        ).as_bytes()).unwrap();
    });

    info!("Aelita fast-forwards staging to master.");
    single_request(&mut github_git_server, |req, mut res| {
        let path = "/repos/AelitaBot/testp/git/refs/heads/staging".to_owned();
        assert_eq!(
            req.uri,
            RequestUri::AbsolutePath(path)
        );
        assert_eq!(
            &req.headers.get_raw("Authorization").unwrap()[0][..],
            b"token MY_PERSONAL_ACCESS_TOKEN"
        );
        *res.status_mut() = StatusCode::Ok;
        res.send(br#""#).unwrap();
    });

    info!("Aelita merges staging to master.");
    single_request(&mut github_git_server, |req, mut res| {
        let path = "/repos/AelitaBot/testp/merges".to_owned();
        assert_eq!(
            req.uri,
            RequestUri::AbsolutePath(path)
        );
        assert_eq!(
            &req.headers.get_raw("Authorization").unwrap()[0][..],
            b"token MY_PERSONAL_ACCESS_TOKEN"
        );
        *res.status_mut() = StatusCode::Ok;
        res.send(concat!(
            r#"{"sha":"#,
                r#""ba218f56b14c9653891f9e74264a383fa43fefbd""#,
            r#"}"#
        ).as_bytes()).unwrap();
    });

    info!("Github sends start notification to Aelita.");
    let http_client = Client::new();
    let mut http_headers = Headers::new();
    http_headers.set_raw("X-Github-Event", vec![b"status".to_vec()]);
    let body = concat!(r#" { "#,
            r#" "state": "pending", "#,
            r#" "target_url": "http://example.com/target_url", "#,
            r#" "context": "ci/test", "#,
            r#" "sha": "ba218f56b14c9653891f9e74264a383fa43fefbd", "#,
            r#" "repository": { "#,
                r#" "name": "testp", "#,
                r#" "owner": {"login":"AelitaBot"} "#,
            r#" } "#,
        r#" } "#
        );
    http_headers.set_raw("X-Hub-Signature", vec![
        format!("sha1={}", openssl::crypto::hmac::hmac(
            openssl::crypto::hash::Type::SHA1,
            "ME_SECRET_LOL".as_bytes(),
            body.as_bytes(),
        ).to_hex()).into_bytes()
    ]);
    http_client.post("http://localhost:9002")
        .body(body.as_bytes())
        .headers(http_headers)
        .send()
        .unwrap();

    info!("Github sends finished notification to Aelita.");
    let http_client = Client::new();
    let mut http_headers = Headers::new();
    http_headers.set_raw("X-Github-Event", vec![b"status".to_vec()]);
    let body = concat!(r#" { "#,
            r#" "state": "success", "#,
            r#" "target_url": "http://example.com/target_url", "#,
            r#" "context": "ci/test", "#,
            r#" "sha": "ba218f56b14c9653891f9e74264a383fa43fefbd", "#,
            r#" "repository": { "#,
                r#" "name": "testp", "#,
                r#" "owner": {"login":"AelitaBot"} "#,
            r#" } "#,
        r#" } "#
        );
    http_headers.set_raw("X-Hub-Signature", vec![
        format!("sha1={}", openssl::crypto::hmac::hmac(
            openssl::crypto::hash::Type::SHA1,
            "ME_SECRET_LOL".as_bytes(),
            body.as_bytes(),
        ).to_hex()).into_bytes()
    ]);
    http_client.post("http://localhost:9002")
        .body(body.as_bytes())
        .headers(http_headers)
        .send()
        .unwrap();

    info!("Aelita marks staging a success.");
    single_request(&mut github_git_server, |req, mut res| {
        let path = format!("/repos/AelitaBot/testp/statuses/{}",
            "ba218f56b14c9653891f9e74264a383fa43fefbd"
        );
        assert_eq!(
            req.uri,
            RequestUri::AbsolutePath(path)
        );
        assert_eq!(
            &req.headers.get_raw("Authorization").unwrap()[0][..],
            b"token MY_PERSONAL_ACCESS_TOKEN"
        );
        #[derive(Deserialize)]
        struct StatusDesc {
            state: String,
            target_url: Option<String>,
            description: String,
            context: String,
        }
        let desc: StatusDesc =
            serde_json::from_reader(req).expect("valid JSON");
        *res.status_mut() = StatusCode::Ok;
        res.send(&[]).unwrap();
        assert_eq!("success", &desc.state[..]);
    });

    info!("Aelita fast-forwards master to staging.");
    let master_string = single_request(&mut github_git_server, |req, mut res| {
        let path = "/repos/AelitaBot/testp/git/refs/heads/master".to_owned();
        assert_eq!(
            req.uri,
            RequestUri::AbsolutePath(path)
        );
        assert_eq!(
            &req.headers.get_raw("Authorization").unwrap()[0][..],
            b"token MY_PERSONAL_ACCESS_TOKEN"
        );
        #[derive(Deserialize)]
        struct FFDesc {
            force: bool,
            sha: String,
        }
        let desc: FFDesc = serde_json::from_reader(req).expect("valid JSON");
        *res.status_mut() = StatusCode::Ok;
        res.send(&[]).unwrap();
        assert!(!desc.force);
        desc.sha
    });
    assert_eq!("ba218f56b14c9653891f9e74264a383fa43fefbd", master_string);

    aelita.kill().unwrap();

    Command::new("/usr/bin/docker")
        .current_dir("./tests/")
        .arg("rm")
        .arg("-f")
        .arg("aelita-test-github-round-trip-postgres")
        .output()
        .unwrap();
}

#[test]
fn one_item_github_round_trip_cloud_with_sqlite_12f() {
    let _lock = ONE_AT_A_TIME.lock();
    START.call_once(|| env_logger::init().unwrap());

    if !Path::new(EXECUTABLE).exists() {
        panic!("Integration tests require the executable to be built.");
    }

    Command::new("/bin/rm")
        .current_dir("./tests/")
        .arg("db.sqlite")
        .output()
        .unwrap();

    let mut github_server = HttpListener::new(&"localhost:9011").unwrap();
    let mut github_git_server = HttpListener::new(&"localhost:9013").unwrap();

    let executable = Path::new(EXECUTABLE).canonicalize().unwrap();
    let mut aelita = Command::new(executable)
        .current_dir("./tests/")
        .arg("-12")
        .env("AELITA_UI_TYPE",
             "github")
        .env("AELITA_PIPELINE_DB",
             "db.sqlite")
        .env("AELITA_PROJECT_DB",
             "db.sqlite")
        .env("AELITA_UI_GITHUB_DB",
             "db.sqlite")
        .env("AELITA_UI_GITHUB_LISTEN",
             "localhost:9001")
        .env("AELITA_UI_GITHUB_HOST",
             "http://localhost:9011")
        .env("AELITA_UI_GITHUB_USER",
             "AelitaBot")
        .env("AELITA_UI_GITHUB_OWNER",
             "AelitaBot")
        .env("AELITA_UI_GITHUB_TOKEN",
             "MY_PERSONAL_ACCESS_TOKEN")
        .env("AELITA_UI_GITHUB_SECRET",
             "ME_SECRET_LOL")
        .env("AELITA_CI_TYPE",
             "github_status")
        .env("AELITA_CI_GITHUB_LISTEN",
             "localhost:9002")
        .env("AELITA_CI_GITHUB_SECRET",
             "ME_SECRET_LOL")
        .env("AELITA_VCS_TYPE",
             "github")
        .env("AELITA_VCS_GITHUB_HOST",
             "http://localhost:9013")
        .env("AELITA_VCS_GITHUB_TOKEN",
             "MY_PERSONAL_ACCESS_TOKEN")
        .env("AELITA_VIEW_LISTEN",
             "localhost:9014")
        .env("AELITA_VIEW_SECRET",
             "your mom")
        .spawn()
        .unwrap();

    info!("Wait a sec for it to finish starting.");
    thread::sleep(time::Duration::new(2, 0));

    info!("Add the pipeline to the database.");
    {
        let conn = rusqlite::Connection::open("./tests/db.sqlite").unwrap();
        conn.execute_batch(r###"
            INSERT INTO twelvef_config_pipeline
                (pipeline_id, name)
            VALUES
                (1, 'Test');
            INSERT INTO twelvef_github_projects
                (pipeline_id, try_pipeline_id, owner, repo)
            VALUES
                (1, NULL, 'AelitaBot', 'testp');
            INSERT INTO twelvef_github_status_pipelines
                (ci_id, owner, repo, context)
            VALUES
                (2, 'AelitaBot', 'testp', 'ci/test');
            INSERT INTO twelvef_config_pipeline_ci
                (ci_id, pipeline_id)
            VALUES
                (2, 1);
            INSERT INTO twelvef_github_git_pipelines
                (
                    pipeline_id,
                    owner,
                    repo,
                    master_branch,
                    staging_branch,
                    push_to_master
                )
            VALUES
                (1, 'AelitaBot', 'testp', 'master', 'staging', 1);
        "###).unwrap();
    }

    info!("Pull request comes into existance.");
    let http_client = Client::new();
    let mut http_headers = Headers::new();
    http_headers.set_raw("X-Github-Event", vec![b"pull_request".to_vec()]);
    let body = concat!(r#" { "#,
        r#" "action":"opened", "#,
        r#" "repository":{ "#,
            r#" "name":"testp", "#,
            r#" "owner":{"login":"AelitaBot","type":"User"} "#,
        r#" }, "#,
        r#" "pull_request":{ "#,
            r#" "title":"HA!", "#,
            r#" "html_url":"http://github.com/testu/testp/pull_request/1", "#,
            r#" "state":"opened", "#,
            r#" "number":1, "#,
            r#" "head":{ "#,
                r#" "sha":"55016813274e906e4cbfed97be83e19e6cd93d91" "#,
            r#" } "#,
        r#" } "#,
    r#" } "#);
    http_headers.set_raw("X-Hub-Signature", vec![
        format!("sha1={}", openssl::crypto::hmac::hmac(
            openssl::crypto::hash::Type::SHA1,
            "ME_SECRET_LOL".as_bytes(),
            body.as_bytes(),
        ).to_hex()).into_bytes()
    ]);
    http_client.post("http://localhost:9001")
        .body(body.as_bytes())
        .headers(http_headers)
        .send()
        .unwrap();

    info!("User posts comment to mark pull request reviewed.");
    let http_client = Client::new();
    let mut http_headers = Headers::new();
    http_headers.set_raw("X-Github-Event", vec![b"issue_comment".to_vec()]);
    let body = concat!(r#" { "#,
        r#" "issue":{ "#,
            r#" "number":1, "#,
            r#" "title":"My PR!", "#,
            r#" "body":"Test", "#,
            r#" "pull_request":{ "#,
                r#" "html_url":"http://github.com/testu/testp/pull_request/1" "#,
            r#" }, "#,
            r#" "state":"opened", "#,
            r#" "user":{ "#,
                r#" "login":"testu", "#,
                r#" "type":"User" "#,
            r#" } "#,
        r#" }, "#,
        r#" "comment":{ "#,
            r#" "user":{ "#,
                r#" "login":"testu", "#,
                r#" "type":"User" "#,
            r#" }, "#,
            r#" "body":"@AelitaBot r+" "#,
        r#" }, "#,
        r#" "repository":{ "#,
            r#" "name":"testp", "#,
            r#" "owner":{ "#,
                r#" "login":"AelitaBot", "#,
                r#" "type":"User" "#,
            r#" } "#,
        r#" } "#,
    r#" } "#);
    http_headers.set_raw("X-Hub-Signature", vec![
        format!("sha1={}", openssl::crypto::hmac::hmac(
            openssl::crypto::hash::Type::SHA1,
            "ME_SECRET_LOL".as_bytes(),
            body.as_bytes(),
        ).to_hex()).into_bytes()
    ]);
    http_client.post("http://localhost:9001")
        .body(body)
        .headers(http_headers)
        .send()
        .unwrap();

    info!("Aelita asks if we're an organization. We're not, for this test.");
    single_request(&mut github_server, |req, mut res| {
        assert_eq!(
            req.uri,
            RequestUri::AbsolutePath("/repos/AelitaBot/testp".to_owned())
        );
        assert_eq!(
            &req.headers.get_raw("Authorization").unwrap()[0][..],
            b"token MY_PERSONAL_ACCESS_TOKEN"
        );
        *res.status_mut() = StatusCode::Ok;
        res.send(concat!(r#" { "#,
            r#" "name":"testp", "#,
            r#" "owner":{"login":"AelitaBot","type":"User"} "#,
            r#" } "#).as_bytes()).unwrap();
    });

    info!("Aelita checks if user has permission to do that.");
    single_request(&mut github_server, |req, mut res| {
        let path = "/repos/AelitaBot/testp/collaborators/testu".to_owned();
        assert_eq!(
            req.uri,
            RequestUri::AbsolutePath(path)
        );
        assert_eq!(
            &req.headers.get_raw("Authorization").unwrap()[0][..],
            b"token MY_PERSONAL_ACCESS_TOKEN"
        );
        *res.status_mut() = StatusCode::NoContent;
        res.send(&[]).unwrap();
    });

    info!("Aelita checks for the current contents of master.");
    single_request(&mut github_git_server, |req, mut res| {
        let path = "/repos/AelitaBot/testp/git/refs/heads/master".to_owned();
        assert_eq!(
            req.uri,
            RequestUri::AbsolutePath(path)
        );
        assert_eq!(
            &req.headers.get_raw("Authorization").unwrap()[0][..],
            b"token MY_PERSONAL_ACCESS_TOKEN"
        );
        *res.status_mut() = StatusCode::Ok;
        res.send(concat!(
            r#"{"object":"#,
                r#"{"sha":"aa218f56b14c9653891f9e74264a383fa43fefbd"}"#,
            r#"}"#
        ).as_bytes()).unwrap();
    });

    info!("Aelita checks for the current contents of staging. They match.");
    single_request(&mut github_git_server, |req, mut res| {
        let path = "/repos/AelitaBot/testp/git/refs/heads/staging".to_owned();
        assert_eq!(
            req.uri,
            RequestUri::AbsolutePath(path)
        );
        assert_eq!(
            &req.headers.get_raw("Authorization").unwrap()[0][..],
            b"token MY_PERSONAL_ACCESS_TOKEN"
        );
        *res.status_mut() = StatusCode::Ok;
        res.send(concat!(
            r#"{"object":"#,
                r#"{"sha":"aa218f56b14c9653891f9e74264a383fa43fefbd"}"#,
            r#"}"#
        ).as_bytes()).unwrap();
    });

    info!("Aelita merges staging to master.");
    single_request(&mut github_git_server, |req, mut res| {
        let path = "/repos/AelitaBot/testp/merges".to_owned();
        assert_eq!(
            req.uri,
            RequestUri::AbsolutePath(path)
        );
        assert_eq!(
            &req.headers.get_raw("Authorization").unwrap()[0][..],
            b"token MY_PERSONAL_ACCESS_TOKEN"
        );
        *res.status_mut() = StatusCode::Ok;
        res.send(concat!(
            r#"{"sha":"#,
                r#""ba218f56b14c9653891f9e74264a383fa43fefbd""#,
            r#"}"#
        ).as_bytes()).unwrap();
    });

    info!("Github sends start notification to Aelita.");
    let http_client = Client::new();
    let mut http_headers = Headers::new();
    http_headers.set_raw("X-Github-Event", vec![b"status".to_vec()]);
    let body = concat!(r#" { "#,
            r#" "state": "pending", "#,
            r#" "target_url": "http://example.com/target_url", "#,
            r#" "context": "ci/test", "#,
            r#" "sha": "ba218f56b14c9653891f9e74264a383fa43fefbd", "#,
            r#" "repository": { "#,
                r#" "name": "testp", "#,
                r#" "owner": {"login":"AelitaBot"} "#,
            r#" } "#,
        r#" } "#
        );
    http_headers.set_raw("X-Hub-Signature", vec![
        format!("sha1={}", openssl::crypto::hmac::hmac(
            openssl::crypto::hash::Type::SHA1,
            "ME_SECRET_LOL".as_bytes(),
            body.as_bytes(),
        ).to_hex()).into_bytes()
    ]);
    http_client.post("http://localhost:9002")
        .body(body.as_bytes())
        .headers(http_headers)
        .send()
        .unwrap();

    info!("Github sends finished notification to Aelita.");
    let http_client = Client::new();
    let mut http_headers = Headers::new();
    http_headers.set_raw("X-Github-Event", vec![b"status".to_vec()]);
    let body = concat!(r#" { "#,
            r#" "state": "success", "#,
            r#" "target_url": "http://example.com/target_url", "#,
            r#" "context": "ci/test", "#,
            r#" "sha": "ba218f56b14c9653891f9e74264a383fa43fefbd", "#,
            r#" "repository": { "#,
                r#" "name": "testp", "#,
                r#" "owner": {"login":"AelitaBot"} "#,
            r#" } "#,
        r#" } "#
        );
    http_headers.set_raw("X-Hub-Signature", vec![
        format!("sha1={}", openssl::crypto::hmac::hmac(
            openssl::crypto::hash::Type::SHA1,
            "ME_SECRET_LOL".as_bytes(),
            body.as_bytes(),
        ).to_hex()).into_bytes()
    ]);
    http_client.post("http://localhost:9002")
        .body(body.as_bytes())
        .headers(http_headers)
        .send()
        .unwrap();

    info!("Aelita marks staging a success.");
    single_request(&mut github_git_server, |req, mut res| {
        let path = format!("/repos/AelitaBot/testp/statuses/{}",
            "ba218f56b14c9653891f9e74264a383fa43fefbd"
        );
        assert_eq!(
            req.uri,
            RequestUri::AbsolutePath(path)
        );
        assert_eq!(
            &req.headers.get_raw("Authorization").unwrap()[0][..],
            b"token MY_PERSONAL_ACCESS_TOKEN"
        );
        #[derive(Deserialize)]
        struct StatusDesc {
            state: String,
            target_url: Option<String>,
            description: String,
            context: String,
        }
        let desc: StatusDesc =
            serde_json::from_reader(req).expect("valid JSON");
        *res.status_mut() = StatusCode::Ok;
        res.send(&[]).unwrap();
        assert_eq!("success", &desc.state[..]);
    });

    info!("Aelita fast-forwards master to staging.");
    let master_string = single_request(&mut github_git_server, |req, mut res| {
        let path = "/repos/AelitaBot/testp/git/refs/heads/master".to_owned();
        assert_eq!(
            req.uri,
            RequestUri::AbsolutePath(path)
        );
        assert_eq!(
            &req.headers.get_raw("Authorization").unwrap()[0][..],
            b"token MY_PERSONAL_ACCESS_TOKEN"
        );
        #[derive(Deserialize)]
        struct FFDesc {
            force: bool,
            sha: String,
        }
        let desc: FFDesc = serde_json::from_reader(req).expect("valid JSON");
        *res.status_mut() = StatusCode::Ok;
        res.send(&[]).unwrap();
        assert!(!desc.force);
        desc.sha
    });
    assert_eq!("ba218f56b14c9653891f9e74264a383fa43fefbd", master_string);

    aelita.kill().unwrap();
}

#[test]
fn test_null_body() {
    let _lock = ONE_AT_A_TIME.lock();
    START.call_once(|| env_logger::init().unwrap());

    if !Path::new(EXECUTABLE).exists() {
        panic!("Integration tests require the executable to be built.");
    }

    Command::new("/bin/rm")
        .current_dir("./tests/")
        .arg("db.sqlite")
        .output()
        .unwrap();

    let mut github_server = HttpListener::new(&"localhost:9011").unwrap();
    let mut jenkins_server = HttpListener::new(&"localhost:9012").unwrap();
    let mut github_git_server = HttpListener::new(&"localhost:9013").unwrap();

    let executable = Path::new(EXECUTABLE).canonicalize().unwrap();
    let mut aelita = Command::new(executable)
        .current_dir("./tests/")
        .arg("test-github-round-trip-cloud.toml")
        .spawn()
        .unwrap();

    info!("Wait a sec for it to finish starting.");
    thread::sleep(time::Duration::new(2, 0));

    info!("Pull request comes into existance.");
    let http_client = Client::new();
    let mut http_headers = Headers::new();
    http_headers.set_raw("X-Github-Event", vec![b"issue_comment".to_vec()]);
    let body = include_str!("test_null_body.json");
    http_headers.set_raw("X-Hub-Signature", vec![
        format!("sha1={}", openssl::crypto::hmac::hmac(
            openssl::crypto::hash::Type::SHA1,
            "ME_SECRET_LOL".as_bytes(),
            body.as_bytes(),
        ).to_hex()).into_bytes()
    ]);
    let result = http_client.post("http://localhost:9001")
        .body(body.as_bytes())
        .headers(http_headers)
        .send()
        .unwrap();

    assert_eq!(true, result.status.is_success());

    aelita.kill().unwrap();
}
