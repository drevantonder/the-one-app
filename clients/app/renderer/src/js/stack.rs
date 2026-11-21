//! A page's error's stack, in the source it was built from: each frame QuickJS says, `at <function> (<file>:<line>:<column>)`,
//! is looked up in its file's sourcemap (`<file>.map`, as an app's build writes one beside each bundle, which a client
//! reads by name), as a browser's devtools and Node's `--enable-source-maps` show it.

use std::collections::HashMap;

/// A frame: its function, if it says one, and where, its file's address, line and column, each from 1.
struct Frame<'a> {
    function: Option<&'a str>,
    file: &'a str,
    line: u32,
    column: u32,
}

fn frame(line: &str) -> Option<Frame<'_>> {
    let said = line.trim().strip_prefix("at ")?;
    let (function, at) = match said.strip_suffix(')').and_then(|said| said.split_once(" (")) {
        Some((function, at)) => (Some(function), at),
        None => (None, said),
    };
    let mut parts = at.rsplitn(3, ':');
    let column = parts.next()?.parse().ok()?;
    let line = parts.next()?.parse().ok()?;
    Some(Frame { function, file: parts.next()?, line, column })
}

/// The files a stack's frames are in, each once: whose sourcemaps `source_mapped` reads.
pub fn stack_files(stack: &str) -> Vec<&str> {
    let mut files: Vec<&str> = stack.lines().filter_map(frame).map(|frame| frame.file).collect();
    files.sort_unstable();
    files.dedup();
    files
}

/// Where a frame is: in the source its file was built from, the source's path with the `../` that led there from where
/// it was built left out (`apps/home-screen/client/pills.tsx`), its line and column, from 1; or, with no sourcemap, in
/// its file, with the function the frame names, which a minifier's renaming makes meaningless where there is one.
pub struct Place {
    pub file: String,
    pub line: u32,
    pub column: u32,
    pub function: Option<String>,
}

/// `stack`'s frames, each in its source where `maps` has its file's sourcemap (`<file>.map`'s bytes, by its file's
/// address), and as it was where it hasn't. → the stack, and the place of its first frame, the one that threw.
pub fn source_mapped(stack: &str, maps: &HashMap<&str, Vec<u8>>) -> (String, Option<Place>) {
    let maps: HashMap<&str, sourcemap::SourceMap> = maps.iter().filter_map(|(file, map)| Some((*file, sourcemap::SourceMap::from_slice(map).ok()?))).collect();
    let mut first = None;
    let lines: Vec<String> = stack
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let Some(at) = frame(line) else { return line.to_owned() };
            let place = match maps.get(at.file).and_then(|map| map.lookup_token(at.line.saturating_sub(1), at.column.saturating_sub(1))) {
                Some(token) => Place {
                    file: token.get_source().unwrap_or(at.file).trim_start_matches("../").to_owned(),
                    line: token.get_src_line() + 1,
                    column: token.get_src_col() + 1,
                    function: None,
                },
                None => Place { file: at.file.to_owned(), line: at.line, column: at.column, function: at.function.map(str::to_owned) },
            };
            let said = match &place.function {
                Some(function) => format!("    at {function} ({}:{}:{})", place.file, place.line, place.column),
                None => format!("    at {}:{}:{}", place.file, place.line, place.column),
            };
            first.get_or_insert(place);
            said
        })
        .collect();
    (lines.join("\n"), first)
}
