//! The one free text insights keeps: what a real failure said (`exception.message`). Cleaned here, the only place it passes
//! through, before anything saves it: what names someone or opens something becomes a placeholder, which also makes errors
//! that differ only by such a value read the same, so they group.

/// The most of what was said that's read, and of what's kept.
const MOST_READ: usize = 8 * 1024;
const MOST_KEPT: usize = 512;

/// Words that say the next word is a secret, as in `token=abc` or `password: hunter2`.
const NAMES_A_SECRET: &[&str] = &["token", "key", "secret", "password", "passwd", "pwd", "auth", "authorization", "bearer", "apikey", "api_key", "cookie", "session"];

const EMAIL: &str = "<email>";
const KEY: &str = "<key>";
const URL: &str = "<url>";
const IP: &str = "<ip>";
const PHONE: &str = "<phone>";
const NUMBER: &str = "<number>";
const PATH: &str = "<path>";

/// `text` with what it mustn't keep as placeholders, and no longer than may be kept.
pub fn cleaned(text: &str) -> String {
    let words = placeholders(cut(text, MOST_READ));
    let numbers = numbers(&words);
    if numbers.len() <= MOST_KEPT { numbers } else { format!("{}…", cut(&numbers, MOST_KEPT)) }
}

/// A short hash of what `cleaned` left, to tell one message from another where it can't be kept.
pub fn id(cleaned: &str) -> String {
    crate::keys::hex(&crate::keys::sha256(cleaned.as_bytes()))[..8].to_string()
}

fn cut(text: &str, most: usize) -> &str {
    let mut end = text.len().min(most);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// What separates words: `:`, `.`, `/` and `@` don't, as addresses and paths are made of them.
fn separates(c: char) -> bool {
    c.is_whitespace() || "=,;()[]{}<>\"'`|".contains(c)
}

fn placeholders(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut secret_next = false;
    let mut rest = text;
    while !rest.is_empty() {
        let (word, after) = rest.split_at(word_end(rest));
        let separator = after.chars().next();
        if !word.is_empty() {
            let bare = word.trim_end_matches(['.', ':', '!', '?']);
            out.push_str(placeholder(bare).or(secret_next.then_some(KEY)).unwrap_or(bare));
            out.push_str(&word[bare.len()..]);
            let lower = bare.to_ascii_lowercase();
            secret_next = NAMES_A_SECRET.contains(&lower.as_str()) && (word.ends_with(':') || separator == Some('=') || lower == "bearer");
        }
        let Some(separator) = separator else { break };
        out.push(separator);
        rest = &after[separator.len_utf8()..];
    }
    out
}

/// Where the word `rest` starts with ends: a URL's runs to the next space, as its query is full of what otherwise separates.
fn word_end(rest: &str) -> usize {
    let end = rest.find(separates).unwrap_or(rest.len());
    if !rest[..end].contains("://") {
        return end;
    }
    let spaced = rest.find(char::is_whitespace).unwrap_or(rest.len());
    rest[..spaced].trim_end_matches(|c| ")]}>'\"`,;.:!?".contains(c)).len()
}

fn placeholder(word: &str) -> Option<&'static str> {
    if word.contains("://") {
        Some(URL)
    } else if email(word) {
        Some(EMAIL)
    } else if home(word) {
        Some(PATH)
    } else if ip(word) {
        Some(IP)
    } else if keyish(word) {
        Some(KEY)
    } else {
        None
    }
}

fn email(word: &str) -> bool {
    word.split_once('@').is_some_and(|(who, domain)| !who.is_empty() && domain.contains('.') && !domain.starts_with('.') && !domain.ends_with('.'))
}

fn home(word: &str) -> bool {
    let lower = word.to_ascii_lowercase().replace('\\', "/");
    ["/home/", "/users/", "/root/", "~/"].iter().any(|at| lower.starts_with(at)) || lower.get(1..).is_some_and(|l| l.starts_with(":/users/"))
}

fn ip(word: &str) -> bool {
    let without_port = word.rsplit_once(':').filter(|(_, port)| !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit())).map_or(word, |(host, _)| host);
    let parts: Vec<&str> = without_port.split('.').collect();
    let v4 = parts.len() == 4 && parts.iter().all(|p| !p.is_empty() && p.len() <= 3 && p.bytes().all(|b| b.is_ascii_digit()));
    let v6 = word.matches(':').count() >= 2 && word.contains(|c: char| c.is_ascii_hexdigit()) && word.bytes().all(|b| b.is_ascii_hexdigit() || b == b':' || b == b'.');
    v4 || v6
}

/// Whether it's a key, a token or an id that's random: long, of a key's alphabet, with both letters and digits, or all hex.
/// A word of a path or a file name is left, so a message can still say where it failed.
fn keyish(word: &str) -> bool {
    if word.starts_with("eyJ") && word.matches('.').count() == 2 {
        return true;
    }
    let of_key = word.bytes().all(|b| b.is_ascii_alphanumeric() || b"_-+=/".contains(&b));
    let digit = word.bytes().any(|b| b.is_ascii_digit());
    let letter = word.bytes().any(|b| b.is_ascii_alphabetic());
    let hex = word.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-');
    let path_like = word.contains('/') && word.len() < 40;
    of_key && !path_like && ((word.len() >= 20 && digit && letter) || (word.len() >= 16 && hex && letter))
}

/// The digits left in `text`: a run of 8 or more with a phone's spaces or marks is a phone number; any other of 4 or
/// more, a long number. Fewer say nothing much: an attempt, a status, a line.
fn numbers(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let starts = c.is_ascii_digit() || ((c == '+' || c == '(') && chars.get(i + 1).is_some_and(char::is_ascii_digit));
        // A digit inside a word (`abc123`, `v2`) is part of it.
        if !starts || (i > 0 && chars[i - 1].is_ascii_alphanumeric()) {
            out.push(c);
            i += 1;
            continue;
        }
        let mut end = i + 1;
        while end < chars.len() && (chars[end].is_ascii_digit() || continues(&chars[end..])) {
            end += 1;
        }
        let run: String = chars[i..end].iter().collect();
        let digits = run.bytes().filter(u8::is_ascii_digit).count();
        let plain = run.bytes().all(|b| b.is_ascii_digit());
        match () {
            _ if digits >= 8 && !plain && !run.contains('.') => out.push_str(PHONE),
            _ if digits >= 4 && plain => out.push_str(NUMBER),
            _ if digits >= 4 && run.starts_with('+') => out.push_str(PHONE),
            _ => out.push_str(&run),
        }
        i = end;
    }
    out
}

/// Whether a phone number's marks, at most two together as in `) `, go on to another of its digits.
fn continues(chars: &[char]) -> bool {
    let marks = chars.iter().take_while(|c| " -().".contains(**c)).count();
    (1..=2).contains(&marks) && chars.get(marks).is_some_and(char::is_ascii_digit)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What names someone, or opens something, never survives; the words around it do, and a message that differs only by
    /// such a value reads the same.
    #[test]
    fn what_would_say_too_much_becomes_a_placeholder() {
        for (said, kept) in [
            ("couldn't mail anna.lee@example.org, try again", "couldn't mail <email>, try again"),
            ("fetching https://x.example/a?token=abc failed", "fetching <url> failed"),
            ("no file at /home/anna/taxes.pdf", "no file at <path>"),
            ("no file at /Users/anna/taxes.pdf.", "no file at <path>."),
            ("the key was wrong (see https://x.example/a?b=c).", "the key was wrong (see <url>)."),
            (r"no file at C:\Users\anna\taxes.pdf", "no file at <path>"),
            ("connecting to 10.0.0.12:8080: refused", "connecting to <ip>: refused"),
            ("connecting to 2001:db8::1 timed out", "connecting to <ip> timed out"),
            ("call +61 400 123 456 back", "call <phone> back"),
            ("call (02) 9876 5432 back", "call <phone> back"),
            ("order 123456789 isn't there", "order <number> isn't there"),
            ("bad key ab12cd34ef56ab12cd34ef56ab12", "bad key <key>"),
            ("token=hunter2 was refused", "token=<key> was refused"),
            ("password: hunter2 was refused", "password: <key> was refused"),
            ("eyJhbGci.eyJzdWIi.c2lnbmF0dXJl expired", "<key> expired"),
            ("attempt 3 of 5 failed with 503 at src/node/calls.rs:123", "attempt 3 of 5 failed with 503 at src/node/calls.rs:123"),
        ] {
            assert_eq!(cleaned(said), kept, "{said}");
        }
        assert_eq!(cleaned("order 123456789 isn't there"), cleaned("order 987654321 isn't there"));
        assert_eq!(id(&cleaned("no mail for a@b.co")), id(&cleaned("no mail for c@d.org")));
        assert_ne!(id(&cleaned("no mail for a@b.co")), id(&cleaned("no calendar for a@b.co")));
        assert!(cleaned(&"é".repeat(MOST_READ)).len() <= MOST_KEPT + "…".len());
    }
}
