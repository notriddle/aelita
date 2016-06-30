// This file is released under the same terms as Rust itself.

use hex::FromHex;
use hyper::server::Request;
use std::io::Read;
use util::crypto::{SHA1_LEN, verify_sha1_hmac};

pub fn parse(req: &mut Request, secret: &[u8]) -> Option<(Vec<u8>, Vec<u8>)> {
    let x_github_event = {
        if let Some(xges) = req.headers.get_raw("X-Github-Event") {
            if let Some(xge) = xges.get(0) {
                xge.clone()
            } else {
                vec![]
            }
        } else {
            vec![]
        }
    };
    let body = {
        let mut body = Vec::new();
        let result = req.read_to_end(&mut body);
        if let Err(e) = result {
            warn!("Failed to read body: {:?}", e);
            return None;
        } else {
            body
        }
    };
    let x_hub_signature = {
        if let Some(xges) = req.headers.get_raw("X-Hub-Signature") {
            if let Some(xge) = xges.get(0) {
                xge.clone()
            } else {
                vec![]
            }
        } else {
            vec![]
        }
    };
    let signature = &x_hub_signature[5..]; // sha1=
    if signature.len() != SHA1_LEN {
        warn!("Got wrong length X-Hub-Signature");
        return None;
    }
    let signature = Vec::from_hex(&signature);
    let signature = if let Ok(signature) = signature {
        signature
    } else {
        warn!("Got invalid hex in X-Hub-Signature");
        return None;
    };
    if !verify_sha1_hmac(secret, &body, &signature) {
        warn!("Got incorrect signature");
        return None;
    }
    Some((x_github_event, body))
}
