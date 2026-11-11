//! What an app runtime's and its services' output goes through to be kept (`super`'s log files).

use std::io::{BufRead, Write};

use super::record;

/// What `tracing` writes to: stderr, and the log.
#[derive(Clone, Copy)]
pub struct Tee;

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Tee {
    type Writer = Tee;
    fn make_writer(&'a self) -> Tee {
        Tee
    }
}

impl Write for Tee {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let _ = std::io::stderr().write_all(buf);
        record(&String::from_utf8_lossy(buf));
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        std::io::stderr().flush()
    }
}

/// Keeps what a program says as it says it (a service's, whose output nobody else reads), and passes it to stderr.
pub fn tee(from: impl std::io::Read + Send + 'static) {
    std::thread::spawn(move || {
        let mut from = std::io::BufReader::new(from);
        let mut line = Vec::new();
        while from.read_until(b'\n', &mut line).is_ok_and(|n| n > 0) {
            let text = String::from_utf8_lossy(&line).to_string();
            eprint!("{text}");
            record(&text);
            line.clear();
        }
    });
}
