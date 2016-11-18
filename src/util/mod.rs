// This file is released under the same terms as Rust itself.

pub mod crypto;
pub mod github_headers;

pub const USER_AGENT: &'static str =
    "aelita/0.1 (https://github.com/AelitaBot/aelita)";

pub const MIN_DELAY_SEC: u64 = 1;
pub const MAX_DELAY_SEC: u64 = 60*2;

macro_rules! retry {
    ($e: expr) => {{
        use std::time::Duration;
        use util::{MIN_DELAY_SEC, MAX_DELAY_SEC};
        let mut delay = Duration::new(MIN_DELAY_SEC / 2, 0);
        let max = Duration::new(MAX_DELAY_SEC, 0);
        loop {
            delay = delay * 2;
            if delay > max {
                panic!("retry loop exceeded maximum delay");
            }
            return $e;
        }
    }}
}

macro_rules! retry_unwrap {
    ($e: expr) => {
        match $e {
            Ok(x) => x,
            Err(e) => {
                warn!("Retrying? {:?}", e);
                continue;
            }
        }
    }
}