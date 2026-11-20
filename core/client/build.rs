use serde_json::Value;

fn main() {
    // The build a program is, as `tooling/version` names it from the commit checked out, which a server and a phone's
    // app are named by too. It's named again whenever the commit does: git's HEAD, or the branch it's on. Not when
    // git's index does, which git writes whenever it looks at the files (`git status`), and every program would be
    // built again for nothing: its `.dirty` is what was uncommitted when the commit last changed. Nor when
    // `packed-refs` does, which a `git fetch` in any worktree of the repo rewrites: a branch that's only in it has no
    // file of its own, which Cargo takes as changed every build.
    let git = |args: &[&str]| {
        let output = std::process::Command::new("git").args(args).output().expect("git is there");
        String::from_utf8(output.stdout).expect("git says text").trim().to_owned()
    };
    let branch = git(&["symbolic-ref", "-q", "HEAD"]);
    for watched in ["HEAD", branch.as_str()].into_iter().filter(|path| !path.is_empty()) {
        println!("cargo:rerun-if-changed={}", git(&["rev-parse", "--path-format=absolute", "--git-path", watched]));
    }
    // A release names it itself where it builds from files a build changes, as a phone's Xcode project is
    // (`tooling/testflight`), from the commit before they're changed: otherwise every such build is `.dirty`.
    println!("cargo:rerun-if-env-changed=ONE_BUILD");
    let build = std::env::var("ONE_BUILD").unwrap_or_else(|_| {
        let named = std::process::Command::new("sh").arg("../../tooling/version").output().expect("tooling/version names the build");
        String::from_utf8(named.stdout).expect("a build's name is text").trim().to_owned()
    });
    println!("cargo:rustc-env=ONE_BUILD={build}");

    // What the app is to the system it's on is its build's Tauri identifier: `clients/app/tauri.conf.json`'s, or the
    // one a build merges over it, as a network's own does with `cargo tauri build --config <its file>`, which hands
    // it on as `TAURI_CONFIG`. It names the folder a client on a computer is kept in (`NAME`), so the CLI, built
    // without Tauri, keeps what the app does.
    println!("cargo:rerun-if-env-changed=TAURI_CONFIG");
    println!("cargo:rerun-if-changed=../../clients/app/tauri.conf.json");
    let identifier = |config: &str| {
        serde_json::from_str::<Value>(config).expect("Tauri's config is JSON")["identifier"].as_str().map(str::to_owned)
    };
    let merged = std::env::var("TAURI_CONFIG").ok().and_then(|config| identifier(&config));
    let base =
        std::fs::read_to_string("../../clients/app/tauri.conf.json").expect("clients/app/tauri.conf.json is there");
    let identifier = merged.or_else(|| identifier(&base)).expect("Tauri's config has an identifier");
    // A debug build is a different app to the system, as Chrome Canary and Electron's and Expo's development builds
    // are: its own folder, never the installed one's, so a build tried from a checkout can't open its person's own
    // client. `ONE_HOME` still says where it's kept instead.
    let identifier = if std::env::var("PROFILE").is_ok_and(|profile| profile == "debug") { format!("{identifier}.dev") } else { identifier };
    println!("cargo:rustc-env=ONE_IDENTIFIER={identifier}");
}
