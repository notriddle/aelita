// This file is released under the same terms as Rust itself.

//! Generic comments-parsing for commands done through text interfaces

use regex::Regex;

lazy_static!{
    static ref REVIEW_BEHALF: Regex = Regex::new(r#"\br=(@?\w+)\b"#)
        .expect("r= is a valid regex");
    static ref REVIEW_SELF: Regex = Regex::new(r#"\br\+(\W|$)"#)
        .expect("r+ is a valid regex");
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Command<'a> {
    Approved(&'a str),
    Canceled,
}

pub fn parse<'a>(body: &'a str, def_user: &'a str) -> Option<Command<'a>> {
    let approved_behalf = parse_approved_behalf(body);
    let approved_default = parse_approved_default(body);
    let canceled = parse_canceled(body);
    match (approved_behalf, approved_default, canceled) {
        (Some(s), false, false) => Some(Command::Approved(s)),
        (None, true, false) => Some(Command::Approved(def_user)),
        (None, false, true) => Some(Command::Canceled),
        _ => None,
    }
}

#[cfg(test)]
mod test {
    use super::*;
    #[test] fn test_comment_body_not_approved() {
        assert_eq!(parse("r?", "luser"), None);
    }
    #[test] fn test_comment_body_approved_def() {
        assert_eq!(parse("r+", "luser"), Some(Command::Approved("luser")));
    }
    #[test] fn test_comment_body_on_behalf() {
        assert_eq!(
            parse("r=genius", "luser"),
            Some(Command::Approved("genius"))
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
            Some(Command::Approved("luser"))
        );
    }
    #[test] fn test_comment_approved_behalf_word_boundary_back() {
        assert_eq!(
            parse("r=genius, thanks!", "luser"),
            Some(Command::Approved("genius"))
        );
    }
    #[test] fn test_comment_approved_behalf_empty() {
        assert_eq!(parse("r= ", "luser"), None);
    }
    #[test] fn test_comment_approved_behalf_at() {
        assert_eq!(
            parse("r=@genius", "luser"),
            Some(Command::Approved("genius"))
        );
    }
    #[test] fn test_comment_approved_word_boundary_back() {
        assert_eq!(parse("r+!", "luser"), Some(Command::Approved("luser")));
    }
    #[test] fn test_comment_approved_word_boundary_front() {
        assert_eq!(parse("!r+", "luser"), Some(Command::Approved("luser")));
    }
}
