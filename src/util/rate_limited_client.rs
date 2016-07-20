// This file is released under the same terms as Rust itself.

use hyper;
use hyper::client::RequestBuilder;
use std::sync::Mutex;
use std::thread;
use std::time;

pub struct RateLimiter {
    rate_limit_until: Mutex<Option<time::SystemTime>>,
}

impl RateLimiter {
    pub fn new() -> RateLimiter {
        RateLimiter {
            rate_limit_until: Mutex::new(None),
        }
    }
    pub fn retry_send<'a, T: FnMut() -> RequestBuilder<'a>>(
        &self,
        mut f: T,
    ) -> hyper::error::Result<hyper::client::Response> {
        const INIT_RETRY_DELAY_MS: u64 = 50;
        const MAX_RETRY_DELAY_SEC: u64 = 60;
        const MIN_RATE_LIMIT_SEC: u64 = 60;
        let mut rate_limit_until = self.rate_limit_until.lock()
            .expect("Rate limit acquire");
        if let Some(ref rate_limit_until) = *rate_limit_until {
            let duration = time::SystemTime::now()
                .duration_since(*rate_limit_until)
                .unwrap_or(time::Duration::new(MIN_RATE_LIMIT_SEC, 0));
            thread::sleep(duration);
        }
        *rate_limit_until = None;
        let mut delay = time::Duration::from_millis(INIT_RETRY_DELAY_MS);
        let mut result = f().send();
        loop {
            result = if let Err(ref e) = result {
                if delay.as_secs() > MAX_RETRY_DELAY_SEC {
                    warn!("HTTP send failed and gave up: {:?}", e);
                    break;
                } else {
                    warn!("HTTP send failed (retry in {:?}): {:?}", delay, e);
                    thread::sleep(delay);
                    delay = delay * 2;
                    f().send()
                }
            } else {
                break;
            }
        }
        if let Ok(ref result) = result {
            let r = result.headers.get_raw("X-RateLimit-Remaining");
            let delayed = if let Some(r) = r {
                let remaining =
                    String::from_utf8_lossy(&r[0])
                    .parse::<u64>()
                    .expect("Github to give me a number here");
                remaining < 2 // let the other thread send a request
            } else {
                false
            };
            if delayed {
                let until = result.headers.get_raw("X-RateLimit-Reset");
                if let Some(until) = until {
                    let until = String::from_utf8_lossy(&until[0])
                        .parse::<u64>()
                        .unwrap_or(0);
                    let until_unix = time::Duration::new(until, 0);
                    let now_unix = (
                        time::SystemTime::now()
                            .duration_since(time::UNIX_EPOCH)
                    ).expect("It's after the unix epoch");
                    let until_delay = if now_unix > until_unix {
                        time::Duration::new(MIN_RATE_LIMIT_SEC, 0)
                    } else {
                        until_unix - now_unix
                    };
                    *rate_limit_until = Some(
                        time::SystemTime::now() + until_delay
                    );
                }
            }
        }
        result
    }
}
