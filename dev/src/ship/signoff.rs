//! Sign-offs: who owns a change says how far it has got, on the repo itself, so any host git can push to carries them and
//! whoever may push may sign. Each is a ref to a small commit on top of the one signed, whose message says what was done.
//! 'tested' is about the code, so every custom build made from it reads the same one; the other stages are about one network.

use super::*;

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Stage {
    Tested,
    Sent,
    Running,
    Seen,
}

impl Stage {
    fn name(self) -> &'static str {
        match self {
            Stage::Tested => "tested",
            Stage::Sent => "sent",
            Stage::Running => "running",
            Stage::Seen => "seen",
        }
    }

    fn folder(self) -> String {
        match self {
            Stage::Tested => "refs/signoff/tested".into(),
            _ => format!("refs/signoff/{CLI}/{}", self.name()),
        }
    }
}

/// The commits each stage is signed for, as the repo has them now.
pub(super) struct Signed(std::collections::HashSet<String>);

impl Signed {
    pub(super) fn read() -> Result<Signed> {
        let out = git(Path::new("."), &["ls-remote", REPO, "refs/signoff/*"])?;
        Ok(Signed(out.lines().filter_map(|l| l.split_once('\t')).map(|(_, name)| name.to_string()).collect()))
    }

    fn has(&self, stage: Stage, commit: &str) -> bool {
        self.0.contains(&format!("{}/{commit}", stage.folder()))
    }

    /// What's left of `commit`, at `index` in `main` (newest first): a later commit's tested, sent or running holds it,
    /// but what someone saw working was that change alone.
    fn next(&self, main: &[String], index: usize) -> Option<Stage> {
        let holds = |stage| main[..=index].iter().any(|c| self.has(stage, c));
        [Stage::Tested, Stage::Sent, Stage::Running].into_iter().find(|stage| !holds(*stage)).or((!self.has(Stage::Seen, &main[index])).then_some(Stage::Seen))
    }
}

/// Signs `commit` off at `stage`, saying `said`. It's already on main.
pub(super) fn record(stage: Stage, commit: &str, said: &str) -> Result<()> {
    keep_history()?;
    let by = devices().into_iter().find(|d| d["here"] == true).and_then(|d| d["name"].as_str().map(str::to_string)).unwrap_or_else(|| "a device".into());
    let message = format!("{}\n\n{said}\nBy: {by}\n", stage.name());
    let history = history();
    // Nothing reads who made this commit, and a builder has no name of its own in git.
    let note = git(&history, &["-c", &format!("user.name={CLI}-dev"), "-c", "user.email=dev@localhost", "commit-tree", &format!("{commit}^{{tree}}"), "-p", commit, "-m", &message])?;
    git(&history, &["push", "-q", "origin", &format!("+{note}:{}/{commit}", stage.folder())])?;
    Ok(())
}

/// `signoff tested`: this checkout's commit passes the checks every change gets, and the agent says which tests it chose.
pub(super) fn tested(ran: &str) -> Result<()> {
    let here = PathBuf::from(git(Path::new("."), &["rev-parse", "--show-toplevel"])?);
    let commit = git(&here, &["rev-parse", "HEAD"])?;
    clean(&here, "commit what's here first: a sign-off is about a commit")?;
    keep_history()?;
    ensure!(on_main(&commit), "{} isn't on main: push it first", short(&commit));
    let signed = Signed::read()?;
    let before = git(&here, &["rev-list", "-n", "1000", "HEAD"])?;
    let base = before.lines().find(|c| signed.has(Stage::Tested, c));
    let files = match base {
        Some(base) => git(&here, &["diff", "--name-only", "--no-renames", base, &commit])?,
        None => git(&here, &["ls-files"])?,
    };
    check(&here, files.lines())?;
    clean(&here, "built files don't match their sources: commit what the build made, push, and sign off that commit")?;
    record(Stage::Tested, &commit, &format!("Ran: {ran}"))?;
    println!("{} is tested: {CLI}-dev ship sends it", short(&commit));
    Ok(())
}

/// The checks every change gets, for what `files` are: it compiles, its screens type-check, and they build as committed.
fn check<'a>(here: &Path, files: impl Iterator<Item = &'a str>) -> Result<()> {
    let files: Vec<&str> = files.collect();
    if files.iter().any(|f| f.ends_with(".rs") || f.ends_with("Cargo.toml") || *f == "Cargo.lock") {
        timed("compiled", || quietly(build::nice_cargo().current_dir(here).args(["check", "--all-targets", "-q"])))?;
    }
    let screen = |f: &&&str| ["src/screen/", "src/onboarding/", "apps/"].iter().any(|top| f.starts_with(top)) && !f.contains("/service/") && !f.contains("/tests/");
    if files.iter().any(|f| screen(&f)) {
        let ui = here.join("src/screen/ui");
        timed("built the screens", || {
            quietly(Command::new("bun").current_dir(&ui).args(["run", "build"]))
        })?;
    }
    for app in Parts::of(files.iter().copied()).apps {
        one::app::deploy::screens(&here.join("apps").join(app))?;
    }
    Ok(())
}

fn clean(here: &Path, or: &str) -> Result<()> {
    let changed = git(here, &["status", "--porcelain"])?;
    ensure!(changed.is_empty(), "{or}:\n{changed}");
    Ok(())
}

fn on_main(commit: &str) -> bool {
    succeeds(git_history().args(["merge-base", "--is-ancestor", commit, "refs/heads/main"]))
}

/// The commit given, wherever this is run from, or this checkout's.
fn named_commit(commit: Option<&str>) -> Result<String> {
    match commit {
        Some(commit) => git(&history(), &["rev-parse", "--verify", &format!("{commit}^{{commit}}")]),
        None => git(Path::new("."), &["rev-parse", "HEAD"]).context("run it in a checkout, or name the commit"),
    }
}

fn short(commit: &str) -> &str {
    &commit[..commit.len().min(12)]
}

/// What an update sends: the newest commit on main with a 'tested' sign-off, saying which changes newer than it stay behind.
pub(super) fn to_send() -> Result<At> {
    let tip = At::at(None)?;
    let signed = Signed::read()?;
    let main: Vec<String> = git(&tip.src, &["rev-list", "-n", "1000", "HEAD"])?.lines().map(str::to_string).collect();
    let Some(newest) = main.iter().position(|c| signed.has(Stage::Tested, c)) else {
        bail!("no commit on main is tested: run {CLI}-dev signoff tested --ran \"<the tests you chose>\" in a checkout of it");
    };
    if newest == 0 {
        return Ok(tip);
    }
    for commit in &main[..newest] {
        let files = git(&tip.src, &["diff-tree", "--no-commit-id", "--name-only", "--no-renames", "-r", commit])?;
        if Parts::of(files.lines()) != Parts::default() {
            println!("Not tested, so staying behind: {}", git(&tip.src, &["log", "-1", "--format=%h %s", commit])?);
        }
    }
    At::at(Some(&main[newest]))
}

/// `ship [<commit>] [--timeout <10m>]`: sends what's tested and returns when every device runs it, or says which doesn't and why.
pub(super) fn ship(commit: Option<&str>, limit: Duration) -> Result<()> {
    keep_history()?;
    let wanted = named_commit(commit)?;
    ensure!(on_main(&wanted), "{} isn't on main: push it first", short(&wanted));
    let at = update(None, false)?;
    ensure!(has(&wanted, &at.build_name(), &mut Default::default()), "{} isn't tested, so it didn't go out: {CLI}-dev signoff tested --ran \"<the tests you chose>\" in a checkout of it", short(&wanted));
    wait_until(&at.build_name(), &[], limit)?;
    keep(Stage::Running, &at.commit, "Every device online then took it");
    prune()?;
    println!("Running everywhere. Once you've seen it work: {CLI}-dev signoff seen {} --checked \"<what you checked>\"", short(&wanted));
    Ok(())
}

/// Signs off what a send did, which has already happened: a builder may read this repo without being able to push to it.
pub(super) fn keep(stage: Stage, commit: &str, said: &str) {
    if let Err(err) = record(stage, commit, said) {
        println!("Couldn't sign {} off as {}: {err:#}", short(commit), stage.name());
    }
}

/// A sign-off only matters while its commit is recent, and fetching from a repo lists every one it has.
fn prune() -> Result<()> {
    let recent: std::collections::HashSet<String> = git(&history(), &["rev-list", "-n", "1000", "refs/heads/main"])?.lines().map(str::to_string).collect();
    let old: Vec<String> = Signed::read()?.0.into_iter().filter(|name| name.rsplit('/').next().is_some_and(|c| !recent.contains(c))).map(|name| format!(":{name}")).collect();
    for batch in old.chunks(200) {
        let mut args = vec!["push", "-q", "origin"];
        args.extend(batch.iter().map(String::as_str));
        git(&history(), &args)?;
    }
    Ok(())
}

/// `signoff seen [<commit>] --checked <what>`: it was seen working, where it went.
pub(super) fn seen(commit: Option<&str>, checked: &str) -> Result<()> {
    keep_history()?;
    let commit = named_commit(commit)?;
    record(Stage::Seen, &commit, &format!("Checked: {checked}"))?;
    println!("{} is finished.", short(&commit));
    Ok(())
}

/// `unfinished [--here]`: changes on main from the last day that their owner hasn't seen through, and what each needs next.
/// With --here, those this checkout made, for an agent's prompt hook: it answers from what the checkout already knows when
/// there are none.
pub(super) fn unfinished(here: bool) -> Result<()> {
    let at = PathBuf::from(git(Path::new("."), &["rev-parse", "--show-toplevel"])?);
    let main: Vec<String> = git(&at, &["rev-list", "-n", "1000", "origin/main"])?.lines().map(str::to_string).collect();
    let recent = git(&at, &["log", "--since=1.day", "--format=%H", "origin/main"])?;
    let made: Vec<&str> = if here {
        let reflog = git(&at, &["log", "-g", "--since=1.day", "--format=%H%x09%gs", "HEAD"])?;
        let mine: std::collections::HashSet<&str> = reflog.lines().filter_map(|l| l.split_once('\t')).filter(|(_, what)| what.starts_with("commit") || what.starts_with("rebase (pick)")).map(|(c, _)| c).collect();
        recent.lines().filter(|c| mine.contains(c)).collect()
    } else {
        recent.lines().collect()
    };
    let shipped: Vec<(&str, usize)> = made.into_iter().filter_map(|c| Some((c, main.iter().position(|m| m == c)?))).collect();
    let due: Vec<(&str, usize)> = shipped.into_iter().filter(|(c, _)| git(&at, &["diff-tree", "--no-commit-id", "--name-only", "--no-renames", "-r", c]).is_ok_and(|f| Parts::of(f.lines()) != Parts::default())).collect();
    if due.is_empty() {
        return Ok(());
    }
    let signed = Signed::read()?;
    let open: Vec<String> = due.into_iter().filter_map(|(c, index)| {
        let stage = signed.next(&main, index)?;
        let next = match stage {
            Stage::Tested => format!("{CLI}-dev signoff tested --ran \"<the tests you chose>\""),
            Stage::Sent | Stage::Running => format!("{CLI}-dev ship {}", short(c)),
            Stage::Seen => format!("{CLI}-dev signoff seen {} --checked \"<what you checked>\"", short(c)),
        };
        Some(format!("{}: not {}: {next}", git(&at, &["log", "-1", "--format=%h %s", c]).ok()?, stage.name()))
    }).collect();
    if !open.is_empty() {
        println!("Unfinished changes:\n{}", open.join("\n"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_later_commit_carries_a_change_out_but_seeing_it_work_is_its_own() {
        let main: Vec<String> = ["c", "b", "a"].map(String::from).to_vec();
        let signed = |names: &[String]| Signed(names.iter().cloned().collect());
        let at = |stage: Stage, commit: &str| format!("{}/{commit}", stage.folder());
        assert!(signed(&[]).next(&main, 2) == Some(Stage::Tested));
        assert!(signed(&[at(Stage::Tested, "c")]).next(&main, 2) == Some(Stage::Sent), "tested on top of it holds it");
        assert!(signed(&[at(Stage::Tested, "a")]).next(&main, 0) == Some(Stage::Tested), "tested beneath it doesn't");
        let out = [at(Stage::Tested, "c"), at(Stage::Sent, "c"), at(Stage::Running, "c")];
        assert!(signed(&out).next(&main, 2) == Some(Stage::Seen), "someone still has to see it");
        let finished = [&out[..], &[at(Stage::Seen, "a")]].concat();
        assert!(signed(&finished).next(&main, 2).is_none());
    }
}
