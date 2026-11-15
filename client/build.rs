// The build a program is: the commit it's built from, as `git describe` names it, with `-dirty` after it when
// what's built isn't all committed. `./release` names it in ONE_BUILD, and a server says its own the same way
// (`server/mix.exs`).
fn main() {
    println!("cargo:rerun-if-env-changed=ONE_BUILD");
    let build = std::env::var("ONE_BUILD").unwrap_or_else(|_| {
        let described = std::process::Command::new("git")
            .args(["describe", "--always", "--dirty", "--abbrev=12", "--exclude=*"])
            .output()
            .expect("git describes what's built");
        String::from_utf8(described.stdout).expect("git says a commit").trim().to_owned()
    });
    println!("cargo:rustc-env=ONE_BUILD={build}");
}
