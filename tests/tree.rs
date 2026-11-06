//! What no commit should have.

/// A merge's markers, left in a tracked file, which `one update` finds only when it ships that file.
#[test]
fn no_file_has_a_conflict_marker_left() {
    let root = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/.."));
    let out = std::process::Command::new("git").args(["ls-files", "--cached", "--exclude-standard"]).current_dir(root).output().expect("git ls-files");
    let left: Vec<String> = String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .filter(|path| {
            let Ok(text) = std::fs::read_to_string(root.join(path)) else { return false };
            text.lines().any(|line| line.starts_with("<<<<<<< ") || line.starts_with(">>>>>>> ") || line == "<<<<<<<" || line == ">>>>>>>")
        })
        .map(str::to_string)
        .collect();
    assert!(left.is_empty(), "a merge's conflict markers are left in: {}", left.join(", "));
}
