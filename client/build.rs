use serde_json::Value;

fn main() {
    // The build a program is: the commit it's built from, as `git describe` names it, with `-dirty` after it when
    // what's built isn't all committed. `./release` names it in ONE_BUILD, and a server says its own the same way
    // (`server/mix.exs`).
    println!("cargo:rerun-if-env-changed=ONE_BUILD");
    let build = std::env::var("ONE_BUILD").unwrap_or_else(|_| {
        let described = std::process::Command::new("git")
            .args(["describe", "--always", "--dirty", "--abbrev=12", "--exclude=*"])
            .output()
            .expect("git describes what's built");
        String::from_utf8(described.stdout).expect("git says a commit").trim().to_owned()
    });
    println!("cargo:rustc-env=ONE_BUILD={build}");

    // What the app is to the system it's on is its build's Tauri identifier: `shell/tauri.conf.json`'s, or the one
    // a build merges over it, as a workplace's own does with `cargo tauri build --config <its file>`, which hands
    // it on as `TAURI_CONFIG`. It names the folder a client on a computer is kept in (`NAME`), so the CLI, built
    // without Tauri, keeps what the app does.
    println!("cargo:rerun-if-env-changed=TAURI_CONFIG");
    println!("cargo:rerun-if-changed=shell/tauri.conf.json");
    let identifier = |config: &str| {
        serde_json::from_str::<Value>(config).expect("Tauri's config is JSON")["identifier"].as_str().map(str::to_owned)
    };
    let merged = std::env::var("TAURI_CONFIG").ok().and_then(|config| identifier(&config));
    let base = std::fs::read_to_string("shell/tauri.conf.json").expect("shell/tauri.conf.json is there");
    let identifier = merged.or_else(|| identifier(&base)).expect("Tauri's config has an identifier");
    println!("cargo:rustc-env=ONE_IDENTIFIER={identifier}");
}
