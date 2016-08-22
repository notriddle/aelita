// This file is released under the same terms as Rust itself.

//! Generic comments-parsing for commands done through text interfaces

use regex::Regex;
use vcs::Commit;

lazy_static!{
    static ref REVIEW_BEHALF: Regex = Regex::new(r#"\br=(@?\w+)\b"#)
        .expect("r= is a valid regex");
    static ref SPECIFIC_COMMIT: Regex = Regex::new(r#"\((\w+)\)"#)
        .expect("(word) is a valid regex");
    static ref REVIEW_SELF: Regex = Regex::new(r#"\br\+(\W|$)"#)
        .expect("r+ is a valid regex");
    static ref TRY_BEHALF: Regex = Regex::new(r#"\btry=(@?\w+)\b"#)
        .expect("try= is a valid regex");
    static ref TRY_SELF: Regex = Regex::new(r#"\btry\+(\W|$)"#)
        .expect("try+ is a valid regex");
    static ref TRY_CANCEL_SELF: Regex = Regex::new(r#"\btry-(\W|$)"#)
        .expect("try- is a valid regex");
    static ref CANCEL_SELF: Regex = Regex::new(r#"\br-(\W|$)"#)
        .expect("r- is a valid regex");
}

fn parse_approved_behalf(body: &str) -> Option<&str> {
    REVIEW_BEHALF.captures(body)
        .and_then(|capture| capture.at(1))
        .map(|username| {
            if username.as_bytes()[0] == b'@' {
                &username[1..]
            } else {
                username
            }
        })
}

fn parse_approved_default(body: &str) -> bool {
    REVIEW_SELF.is_match(body)
}

fn parse_canceled(body: &str) -> bool {
    CANCEL_SELF.is_match(body)
}

fn parse_try_approved_behalf(body: &str) -> Option<&str> {
    TRY_BEHALF.captures(body)
        .and_then(|capture| capture.at(1))
        .map(|username| {
            if username.as_bytes()[0] == b'@' {
                &username[1..]
            } else {
                username
            }
        })
}

fn parse_try_approved_default(body: &str) -> bool {
    TRY_SELF.is_match(body)
}

fn parse_try_canceled(body: &str) -> bool {
    TRY_CANCEL_SELF.is_match(body)
}

fn parse_specific_commit<C: Commit>(body: &str) -> Option<C> {
    SPECIFIC_COMMIT.captures(body)
        .and_then(|capture| capture.at(1))
        .and_then(|commit| C::from_str(commit).ok())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Command<'a, C: Commit> {
    Approved(&'a str, Option<C>),
    Canceled,
    TryApproved(&'a str, Option<C>),
    TryCanceled,
}

pub fn parse<'a, C>(body: &'a str, def_user: &'a str) -> Option<Command<'a, C>>
    where C: Commit
{
    let approved_behalf = parse_approved_behalf(body);
    let approved_default = parse_approved_default(body);
    let canceled = parse_canceled(body);
    let commit = parse_specific_commit(body);
    let try_behalf = parse_try_approved_behalf(body);
    let try_approved_default = parse_try_approved_default(body);
    let try_canceled = parse_try_canceled(body);
    match (
        approved_behalf,
        approved_default,
        canceled,
        try_behalf,
        try_approved_default,
        try_canceled,
    ) {
        (Some(user), false, false, None,       false, false) =>
            Some(Command::Approved(user, commit)),
        (None,       true, false, None,        false, false) =>
            Some(Command::Approved(def_user, commit)),
        (None,       false, true, None,        false, false) =>
            Some(Command::Canceled),
        (None,       false, false, Some(user), false, false) =>
            Some(Command::TryApproved(user, commit)),
        (None,       false, false, None,       true, false) =>
            Some(Command::TryApproved(def_user, commit)),
        (None,       false, false, None,       false, true) =>
            Some(Command::TryCanceled),
        _ => None,
    }
}

#[cfg(test)]
mod test {
    use std::str::FromStr;
    use super::Command;
    use vcs::git;
    fn parse<'a>(body: &'a str, def_user: &'a str)
        -> Option<Command<'a, git::Commit>> {
        super::parse(body, def_user)
    }
    #[test] fn test_empty_comment() {
        assert_eq!(parse("", ""), None);
    }
    #[test] fn test_comment_body_not_approved() {
        assert_eq!(parse("r?", "luser"), None);
    }
    #[test] fn test_comment_body_approved_def() {
        assert_eq!(
            parse("r+", "luser"),
            Some(Command::Approved("luser", None))
        );
    }
    #[test] fn test_comment_body_on_behalf() {
        assert_eq!(
            parse("r=genius", "luser"),
            Some(Command::Approved("genius", None))
        );
    }
    #[test] fn test_comment_cancel() {
        assert_eq!(parse("r-", "luser"), Some(Command::Canceled));
    }
    #[test] fn test_comment_not_approved_substr() {
        assert_eq!(parse("ear+", "luser"), None);
    }
    #[test] fn test_comment_not_approved_behalf_substr() {
        assert_eq!(parse("ear=nose", "luser"), None);
    }
    #[test] fn test_comment_not_canceled_substr() {
        assert_eq!(parse("ear-", "luser"), None);
    }
    #[test] fn test_comment_not_canceled_substr_back() {
        assert_eq!(parse("r-a=q", "luser"), None);
    }
    #[test] fn test_comment_not_approved_substr_back() {
        assert_eq!(parse("r+a=q", "luser"), None);
    }
    #[test] fn test_comment_approved_space_back() {
        assert_eq!(
            parse("r+ is not a license to kill", "luser"),
            Some(Command::Approved("luser", None))
        );
    }
    #[test] fn test_comment_approved_behalf_word_boundary_back() {
        assert_eq!(
            parse("r=genius, thanks!", "luser"),
            Some(Command::Approved("genius", None))
        );
    }
    #[test] fn test_comment_approved_behalf_empty() {
        assert_eq!(parse("r= ", "luser"), None);
    }
    #[test] fn test_comment_approved_behalf_at() {
        assert_eq!(
            parse("r=@genius", "luser"),
            Some(Command::Approved("genius", None))
        );
    }
    #[test] fn test_comment_approved_word_boundary_back() {
        assert_eq!(
            parse("r+!", "luser"),
            Some(Command::Approved("luser", None))
        );
    }
    #[test] fn test_comment_approved_word_boundary_front() {
        assert_eq!(
            parse("!r+", "luser"),
            Some(Command::Approved("luser", None))
        );
    }
    #[test] fn test_comment_approved_specific_commit() {
        assert_eq!(
            parse("r+ (a4068472538866d0b603793539875dac1f962c2e)", "luser"),
            Some(Command::Approved(
                "luser",
                Some(git::Commit::from_str(
                    "a4068472538866d0b603793539875dac1f962c2e"
                ).unwrap())
            ))
        );
    }
    #[test] fn test_try() {
        assert_eq!(
            parse("try+", "luser"),
            Some(Command::TryApproved("luser", None))
        );
    }
    #[test] fn test_try_cancel() {
        assert_eq!(
            parse("try-", "luser"),
            Some(Command::TryCanceled)
        );
    }
}
