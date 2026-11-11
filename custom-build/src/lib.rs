//! What a custom build is (GLOSSARY): config.toml's constants (`config`), the names of its programs (`programs`), and
//! what its machines and languages say to each other the same way (`protocol`).

pub mod programs;
pub mod protocol;

pub mod config {
    include!(concat!(env!("OUT_DIR"), "/config.rs"));

    /// The environment variable this build's programs read for `suffix`: its CLI's name in capitals, then `_<suffix>`
    /// (`ONE_HOME`), with a hyphen in the name an underscore.
    pub fn env(suffix: &str) -> String {
        format!("{}_{suffix}", CLI.to_uppercase().replace('-', "_"))
    }
}

/// `s` as a part of a URL: its unreserved characters as they are, every other byte percent-encoded.
pub fn url_encode(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64
}
