//! The app runtime's program (`<cli>-apps`), which oned starts as `<cli>-apps --listen=<address>` (`app_runtime::serve`)
//! and asks which build it is as `version --json`, as it asks any program it runs.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let done = match args.as_slice() {
        ["version", "--json"] => Ok(println!("{}", ::oned_update::release::Version::this().json())),
        [listen] if let Some(listen) = listen.strip_prefix("--listen=") => app_runtime::serve(listen),
        _ => Err(anyhow::anyhow!("this is the app runtime {} starts (--listen=<address>); version --json says which build it is", ::custom_build::programs::oned())),
    };
    if let Err(err) = done {
        eprintln!("{}-apps: {err:#}", ::custom_build::config::CLI);
        std::process::exit(1);
    }
}
