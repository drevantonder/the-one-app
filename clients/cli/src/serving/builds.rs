//! Where the builds come from: downloaded, checked against the platform's signing key and put in place.

use super::*;

/// Downloads the build `one` of the program `program` beside it, and gives where, once it has said it's that build.
/// It's beside the file a link to it names, as on a Mac `~/.local/bin/one` is a link to the program in the One App's
/// bundle, under a name of this process's own (`one.<pid>.new`): the app and the server's `one serve run` each take
/// the same program at once, which on Windows one couldn't write while the other held it (os error 32).
pub(super) async fn fetched_program(one: &Value, program: &Path) -> Result<PathBuf> {
    let build = one["build"].as_str().context("a build says which it is")?;
    ensure!(count(build) >= count(BUILD), "The network's build of one, {build}, is older than this one, {BUILD}.");
    let packed = program.canonicalize()?.with_extension(format!("{}.new.gz", std::process::id()));
    fetch(&name(), one, &packed).await?;
    let new = packed.with_extension("");
    let unpacked =
        std::io::copy(&mut flate2::read::GzDecoder::new(fs::File::open(&packed)?), &mut fs::File::create(&new)?);
    fs::remove_file(&packed)?;
    unpacked.with_context(|| format!("The build {build} of one didn't unpack."))?;
    #[cfg(unix)]
    fs::set_permissions(&new, std::os::unix::fs::PermissionsExt::from_mode(0o755))?;
    #[cfg(target_os = "macos")]
    let started = bundled(&new)?;
    #[cfg(not(target_os = "macos"))]
    let started = build_of(&new);
    if started.as_deref() != Some(build) {
        fs::remove_file(&new)?;
        bail!("The build {build} of one doesn't start here.")
    }
    Ok(new)
}

/// A Mac's app bundle but its program: its Info.plist, icon and seal, the same for every build, which each build of
/// `one` is signed in (`tooling/release`), so an update replaces the program alone, as before it was signed. macOS
/// runs a program signed in a bundle only as that bundle's program, and kills it anywhere else in it, or in a bundle
/// that isn't the one it was signed in. A change to these files goes out in a build signed without them first.
#[cfg(target_os = "macos")]
pub(super) const BUNDLE: &[(&str, &[u8])] = &[
    ("Info.plist", include_bytes!("../../../app/macos/Contents/Info.plist")),
    ("Resources/The One App.icns", include_bytes!("../../../app/macos/Contents/Resources/The One App.icns")),
    ("_CodeSignature/CodeResources", include_bytes!("../../../app/macos/Contents/_CodeSignature/CodeResources")),
];

/// The build the program `new` says it is (`build_of`), downloaded beside a Mac's: where that's in an app bundle,
/// it's run as the program of a bundle of its own inside it (`Contents/Updates/<pid>.app`), and `BUNDLE` is put in
/// place around both, so it starts there, and as the bundle's program once it's put in place.
#[cfg(target_os = "macos")]
pub(super) fn bundled(new: &Path) -> Result<Option<String>> {
    let Some(contents) = new.parent().filter(|folder| folder.ends_with("Contents/MacOS")).and_then(Path::parent) else {
        return Ok(build_of(new));
    };
    let updates = contents.join("Updates");
    let checking = updates.join(format!("{}.app/Contents", std::process::id()));
    for folder in [contents, checking.as_path()] {
        for (file, bytes) in BUNDLE {
            let path = folder.join(file);
            if fs::read(&path).is_ok_and(|was| was == *bytes) {
                continue;
            }
            fs::create_dir_all(path.parent().context("a file is in a folder")?)?;
            let writing = path.with_extension(format!("{}.new", std::process::id()));
            fs::write(&writing, bytes)?;
            fs::rename(&writing, &path)?;
        }
    }
    let program = checking.join("MacOS/one");
    fs::create_dir_all(checking.join("MacOS"))?;
    fs::rename(new, &program)?;
    let build = build_of(&program);
    fs::rename(&program, new)?;
    fs::remove_dir_all(checking.parent().context("a bundle is in a folder")?)?;
    // Another's may be checking beside it, as the app and the server take a build at once.
    let _ = fs::remove_dir(&updates);
    Ok(build)
}

/// The build the program `program` says it is (`one --version`), if it starts.
pub fn build_of(program: &Path) -> Option<String> {
    let output = command(program).arg("--version").output().ok()?;
    String::from_utf8_lossy(&output.stdout).split_whitespace().nth(1).map(str::to_owned)
}

/// Puts the program `new` in place of `program`, in one step. Windows replaces or removes no program that's running,
/// but lets it be renamed: the one before is put aside under a name of its own (`one.<ms>.old`), as one put aside
/// before may still be running (the app, or the `one serve run` that's putting this in place, started from it), and
/// those put aside before that nothing runs any more go.
pub(super) fn put(new: &Path, program: &Path) -> Result<()> {
    let program = program.canonicalize()?;
    #[cfg(windows)]
    {
        let stem = program.file_stem().unwrap_or_default().to_string_lossy().into_owned();
        let folder = program.parent().context("a program is in a folder")?;
        for aside in fs::read_dir(folder)?.flatten().map(|entry| entry.path()) {
            let name = aside.file_name().unwrap_or_default().to_string_lossy();
            if name.starts_with(&format!("{stem}.")) && name.ends_with(".old") {
                let _ = fs::remove_file(&aside);
            }
        }
        fs::rename(&program, program.with_extension(format!("{}.old", now())))?;
    }
    Ok(fs::rename(new, program)?)
}

/// What starts a build of the server: its release's script, a batch file on Windows.
pub(super) fn release(build: &Path) -> PathBuf {
    build.join(if cfg!(windows) { "bin/one_core.bat" } else { "bin/one_core" })
}

/// A program this one starts, which has no window of its own: on Windows a console program started by one with no
/// console, as a server's is, would open one.
pub(super) fn command(program: impl AsRef<std::ffi::OsStr>) -> std::process::Command {
    #[allow(unused_mut)]
    let mut command = std::process::Command::new(program);
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(&mut command, 0x0800_0000); // CREATE_NO_WINDOW
    command
}

/// The latest builds for this kind of machine, by what they are (`server`, `one`), each with a link to it.
pub(super) async fn download(client: &Client) -> Result<Map<String, Value>> {
    let response = tool(client, "download", json!({"target": target()})).await?;
    match serde_json::from_str(&response)? {
        Value::Object(builds) => Ok(builds),
        _ => bail!("the network didn't list its builds"),
    }
}

/// Downloads the build of `name` the network gives, `build`, by its link into `file`, once it's checked it's the
/// platform's, and checks it's the size and SHA-256 it said. A download whose connection fails is tried again, after
/// 2 s and then 4 s, as curl's `--retry` does: a home router's NAT, or GitHub, now and then drops a connection, and one
/// dropped would leave the server behind until the next build.
pub(super) async fn fetch(name: &str, build: &Value, file: &Path) -> Result<()> {
    signed(name, build)?;
    for wait in [2, 4] {
        match download_to(build, file).await {
            Err(error) if error.is::<reqwest::Error>() => {
                eprintln!("The build {} didn't download ({error:#}); trying again in {wait} s", build["build"]);
                tokio::time::sleep(Duration::from_secs(wait)).await;
            }
            done => return done,
        }
    }
    download_to(build, file).await
}

/// Downloads `build` by its link into `file`, and checks it's the size and SHA-256 it said.
pub(super) async fn download_to(build: &Value, file: &Path) -> Result<()> {
    let link = &build["link"];
    let method = reqwest::Method::from_bytes(link["method"].as_str().unwrap_or("GET").as_bytes())?;
    let mut request = client::web()?.request(method, link["url"].as_str().context("a build has a link")?);
    for (name, value) in link["headers"].as_object().into_iter().flatten() {
        request = request.header(name, value.as_str().unwrap_or_default());
    }
    let mut response = request.send().await?.error_for_status()?;
    let mut out = tokio::fs::File::create(file).await?;
    let (mut hash, mut size) = (Sha256::new(), 0);
    while let Some(part) = response.chunk().await? {
        hash.update(&part);
        size += part.len() as u64;
        out.write_all(&part).await?;
    }
    out.flush().await?;
    if build["sha256"] != data_encoding::HEXLOWER.encode(&hash.finalize()) || build["size"] != size {
        fs::remove_file(file)?;
        bail!("The build {} didn't download whole.", build["build"]);
    }
    Ok(())
}

/// Checks the network's `build` of `name` (`one/linux-x86_64`, `server-code`) is the platform's: its Ed25519 signature
/// of its name, build and SHA-256, checked with the public half built in here, `KEY`, as Tauri's updater and Sparkle
/// check theirs. So whoever publishes to the network, its owner too, can give this no program but the platform's, nor
/// one in another's place, and as each says its build, an older one is told by it.
pub(super) fn signed(name: &str, build: &Value) -> Result<()> {
    let which = build["build"].as_str().context("a build says which it is")?;
    let said = format!("{name} {which} {}", build["sha256"].as_str().context("a build says its SHA-256")?);
    let signature = build["signature"].as_str().with_context(|| format!("The build {which} of {name} isn't signed."))?;
    let signature = data_encoding::BASE64.decode(signature.as_bytes()).ok().and_then(|sig| Signature::from_slice(&sig).ok());
    let key = data_encoding::HEXLOWER.decode(KEY.as_bytes()).ok().and_then(|key| VerifyingKey::try_from(key.as_slice()).ok());
    let signed = signature.is_some_and(|sig| key.expect("KEY is a public key").verify_strict(said.as_bytes(), &sig).is_ok());
    ensure!(signed, "The build {which} of {name} isn't signed by the platform.");
    Ok(())
}

/// The count of commits in a build's metadata (`tooling/version`), which only grows on main: `0.25.13+3670.9046435e40b3`
/// is 3670, as `OneCore.Builds.count/1` reads it.
pub fn count(build: &str) -> u64 {
    build.split_once('+').and_then(|(_, metadata)| metadata.split('.').next()?.parse().ok()).unwrap_or(0)
}

/// Calls one of the network's tools and gives what it said, or the error it said.
pub(super) async fn tool(client: &Client, name: &str, arguments: Value) -> Result<String> {
    crate::said(client.request("tools/call", json!({"name": name, "arguments": arguments})).await?)
}
