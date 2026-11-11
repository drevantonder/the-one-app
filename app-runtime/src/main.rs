//! The app runtime's program (`<cli>-apps`), which oned starts serving a loopback socket it keeps, `<cli>-apps --listen-socket=<it>` (`app_runtime::serve`)
//! and asks which build it is as `version --json`, as it asks any program it runs.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let done = match args.as_slice() {
        ["version", "--json"] => Ok(println!("{}", ::oned_update::release::Version::this().json())),
        [socket] if let Some(socket) = socket.strip_prefix("--listen-socket=") => platform::os::handed_listener(socket).and_then(app_runtime::serve),
        _ => Err(anyhow::anyhow!("this is the app runtime {} starts (--listen-socket=<socket>); version --json says which build it is", ::custom_build::programs::oned())),
    };
    if let Err(err) = done {
        eprintln!("{}-apps: {err:#}", ::custom_build::config::CLI);
        std::process::exit(1);
    }
}
