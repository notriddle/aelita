// This file is released under the same terms as Rust itself.

pub mod crypto;
pub mod github_headers;

pub const USER_AGENT: &'static str =
    "aelita/0.1 (https://github.com/AelitaBot/aelita)";

pub const MIN_DELAY_SEC: u64 = 1;
pub const MAX_DELAY_SEC: u64 = 60*2;
