//! `symbols <sample>`: a Mac stall's stacks (`logs/stall.sample`, written by the watchdog) as function names. Release
//! programs ship stripped, and the Mac's build keeps each one's symbols beside its cache (`client/native/mac/build.sh`),
//! found by the program's UUID, so a sample always gets the symbols of the very build it came from.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Result, ensure};
use one::cli::{Command as Cli, Failed};
use one::config::CLI;

pub fn command() -> Cli {
    Cli {
        help: vec![("symbols <file>".into(), format!("read a Mac's stall.sample (under {CLI}'s home, logs/) with its ??? frames named, as the Mac's build kept each release's symbols: run it on the Mac that built that release (its checkout's {CLI}-dev)"))],
        run: |g| {
            let words = g.words();
            let ["symbols", file] = words.as_slice() else { return Ok(false) };
            g.takes_only(&[])?;
            print!("{}", resolved(&std::fs::read_to_string(file).map_err(|e| Failed::Typed(format!("couldn't read {file}: {e}")))?, &symbols_dir())?);
            Ok(true)
        },
    }
}

/// Where the Mac's build keeps each program's symbols (build.sh says the same).
fn symbols_dir() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join("Library/Caches").join(CLI).join("symbols")
}

/// A sample's stacks, each frame the symbols of its own program found by name instead of `???`. What it can't name stays.
fn resolved(sample: &str, dir: &Path) -> Result<String> {
    let images = images(sample);
    let mut tables: std::collections::HashMap<String, Option<Table>> = Default::default();
    let mut out = String::with_capacity(sample.len());
    let (mut named, mut frames) = (0, 0);
    for line in sample.lines() {
        let frame = parsed(line).and_then(|(prefix, image, offset)| Some((prefix, image, offset, images.get(image)?)));
        let said = frame.and_then(|(prefix, image, offset, uuid)| {
            frames += 1;
            let table = tables.entry(uuid.clone()).or_insert_with(|| Table::read(&dir.join(format!("{uuid}.gz")))).as_ref()?;
            let name = table.name_at(offset)?;
            named += 1;
            Some(format!("{prefix}{name}  (in {image})"))
        });
        out.push_str(said.as_deref().unwrap_or(line));
        out.push('\n');
    }
    ensure!(frames == 0 || named > 0, "no symbols for this sample in {}: they're kept on the Mac that built the release it came from, by its UUID", dir.display());
    Ok(out)
}

/// A `sample` frame of a stripped program: what comes before its function, its image, and where in it.
fn parsed(line: &str) -> Option<(&str, &str, u64)> {
    let (prefix, rest) = line.split_once("???  (in ")?;
    let (image, rest) = rest.split_once(")  load address ")?;
    let (_, offset) = rest.split_once(" + 0x")?;
    Some((prefix, image, u64::from_str_radix(offset.split_whitespace().next()?, 16).ok()?))
}

/// Each image in a sample's list, by name: the UUID of what ran, as the Mac's build kept its symbols.
fn images(sample: &str) -> std::collections::HashMap<&str, String> {
    let list = sample.split_once("Binary Images:").map_or("", |(_, list)| list);
    list.lines().filter_map(|line| {
        let (before, after) = line.split_once('<')?;
        let uuid = after.split_once('>')?.0.to_string();
        let name = before.trim().split_whitespace().nth(3)?.trim_start_matches('+');
        Some((name, uuid))
    }).collect()
}

/// A program's functions by where they start in it, offsets from its first symbol: what `nm -n` says.
struct Table(Vec<(u64, String)>);

impl Table {
    fn read(file: &Path) -> Option<Table> {
        let unpacked = Command::new("gzip").arg("-dc").arg(file).output().ok().filter(|o| o.status.success())?;
        let text = String::from_utf8(unpacked.stdout).ok()?;
        let base = text.lines().find_map(|l| l.split_once(' ')).and_then(|(address, _)| u64::from_str_radix(address, 16).ok())?;
        Some(Table(text.lines().filter_map(|l| {
            let (address, name) = l.split_once(' ')?;
            Some((u64::from_str_radix(address, 16).ok()?.checked_sub(base)?, name.to_string()))
        }).collect()))
    }

    fn name_at(&self, offset: u64) -> Option<String> {
        let at = self.0.partition_point(|(start, _)| *start <= offset);
        let (start, name) = self.0.get(at.checked_sub(1)?)?;
        Some(format!("{} + {}", demangled(name), offset - start))
    }
}

/// A symbol as it was written. `nm` on a Mac shows each with one more `_`.
fn demangled(symbol: &str) -> String {
    let name = symbol.strip_prefix('_').unwrap_or(symbol);
    rustc_demangle::try_demangle(name).map_or_else(|_| name.to_string(), |d| format!("{d:#}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_macs_stacks_are_named_by_the_symbols_of_the_same_program() {
        let dir = std::env::temp_dir().join(format!("symbols-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let table = "0000000100000000 __mh_execute_header\n0000000100069e00 __ZN3one4node5serve17h0123456789abcdefE\n";
        let file = dir.join("2FB9DA3B.gz");
        let mut gzip = Command::new("gzip").arg("-c").stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).spawn().unwrap();
        use std::io::Write;
        gzip.stdin.take().unwrap().write_all(table.as_bytes()).unwrap();
        std::fs::write(&file, gzip.wait_with_output().unwrap().stdout).unwrap();
        let sample = "    +   667 ???  (in one)  load address 0x1025b4000 + 0x69e94  [0x10261de94]\n    +   ??? (in libsystem)\nBinary Images:\n  0x1025b4000 -  0x10535f70b +one (0) <2FB9DA3B> /Users/x/one\n";
        let read = resolved(sample, &dir).unwrap();
        assert!(read.contains("one::node::serve + 148  (in one)"), "{read}");
        assert!(read.contains("+   ??? (in libsystem)"), "what isn't the program's stays: {read}");
        assert_eq!(demangled("__ZN4core3fmt5write17h0123456789abcdefE"), "core::fmt::write");
        assert_eq!(demangled("__RNvNtCs1234_4core3fmt5write"), "core::fmt::write");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
