//! An app's service, built and published for every platform by an update, or put on one device by `services try`.

use super::*;

/// The app `name`'s service's source in `src`.
pub(super) fn service_dir(src: &Path, name: &str) -> PathBuf {
    src.join("apps").join(name).join("service")
}

/// The apps in `src` with a service: each `apps/<name>/service/` with a Go module or a Rust package.
pub(super) fn services(src: &Path) -> Vec<String> {
    let has = |e: &std::fs::DirEntry| ["go.mod", "Cargo.toml"].iter().any(|f| e.path().join("service").join(f).is_file());
    let mut names: Vec<String> = std::fs::read_dir(src.join("apps")).into_iter().flatten().flatten().filter(has).map(|e| e.file_name().to_string_lossy().to_string()).collect();
    names.sort();
    names
}

/// Puts your checkout's build of the app's service in `dir` (`apps/<name>`) on `device` (here by default, else reached
/// by ssh as its name) and switches it on there, for trying it before it's sent out. oned runs it until a release of it
/// newer than this build is published.
pub fn try_service(dir: &Path, device: Option<&str>) -> Result<()> {
    let dir = std::fs::canonicalize(dir).with_context(|| format!("{} isn't here", dir.display()))?;
    let name = dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    ensure!(build::has_service(&dir), "{} has no service (service/go.mod or service/Cargo.toml)", dir.display());
    let on = |args: &str| -> Result<String> {
        let out = match device {
            Some(device) => Command::new("ssh").args([device, args]).stdin(std::process::Stdio::null()).output(),
            None => Command::new("sh").args(["-c", args]).output(),
        }?;
        ensure!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr).trim());
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    };
    let there = device.unwrap_or("this device");
    on(&format!("test -x ~/.local/bin/{CLI}")).map_err(|_| anyhow::anyhow!("{CLI} isn't on {there}: install it there first"))?;
    let uname = on("uname -sm")?;
    let platform = Platform::of_uname(&uname).with_context(|| format!("{there} says it's {uname}, which no release is for"))?;
    let built = build::built_from(&dir.join("service"), true)?;
    let out = std::env::temp_dir().join(format!("{CLI}-{name}-{}", std::process::id()));
    let made = build::build_service(&dir, &built, [platform], &out);
    let made = made.and_then(|b| match b {
        Some(b) if !b.made.is_empty() => Ok(b.made[0].1.clone()),
        _ => bail!("a service in Rust is built here for this machine, so try it on a device like it ({TARGET}) from one"),
    });
    let program = match made {
        Ok(program) => program,
        Err(err) => {
            let _ = std::fs::remove_dir_all(&out);
            return Err(err);
        }
    };
    // Put in place in one step, so oned never starts half of it. Here, where oned looks for it (`oned::fetched`: a throwaway
    // network's own, never the real network's); on another device, in its `~/.local/bin`.
    let installed = format!("{CLI}-{name}");
    let copied = match device {
        Some(device) => succeeds(Command::new("scp").arg("-q").arg(&program).arg(format!("{device}:.local/bin/{installed}.new"))),
        None => {
            let new = one::oned::fetched(&name).with_extension("new");
            new.parent().is_some_and(|dir| std::fs::create_dir_all(dir).is_ok()) && std::fs::copy(&program, new).is_ok()
        }
    };
    let _ = std::fs::remove_dir_all(&out);
    ensure!(copied, "couldn't copy it to {there}");
    match device {
        Some(_) => drop(on(&format!("chmod 755 ~/.local/bin/{installed}.new && mv ~/.local/bin/{installed}.new ~/.local/bin/{installed}"))?),
        None => {
            use std::os::unix::fs::PermissionsExt;
            let path = one::oned::fetched(&name);
            std::fs::set_permissions(path.with_extension("new"), std::fs::Permissions::from_mode(0o755))?;
            std::fs::rename(path.with_extension("new"), path)?;
        }
    }
    crate::node().post("/v1/service", json!({ "name": name, "device": device, "on": true }))?;
    println!("{name}'s service {built} is on {there}: its oned starts it; {CLI} services says when it's running.");
    Ok(())
}

/// Each service's program last published from here, and the commit it was built from.
pub(super) fn services_released() -> serde_json::Map<String, Value> {
    std::fs::read(cache().join("services-released")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// Builds each service named for every platform the installer knows, signs it and publishes it, so each device that
/// runs it takes it (and one that's switched on there fetches it). → false if one wasn't.
pub(super) fn publish_services(src: &Path, names: &[String], key: &ssh_key::PrivateKey, commit: &str) -> bool {
    let mut ok = true;
    let committed: u64 = git(src, &["log", "-1", "--format=%ct", commit]).ok().and_then(|t| t.trim().parse().ok()).unwrap_or(0);
    let mut released = services_released();
    for name in names {
        let each = || -> Result<()> {
            // One in Rust is built for this machine's platform: each Mac builds its own with its app, and carries it.
            let out = cache().join("services").join(name);
            let built = build::build_service(&src.join("apps").join(name), &commit[..12], PLATFORMS.iter(), &out)?.context("it has no service")?;
            for (platform, program) in built.made {
                let version = Version { commit: commit.into(), target: platform.target.into(), committed };
                build::publish(&crate::node(), &program, key, &version, Some(name)).with_context(|| format!("publish it for {}", platform.target))?;
            }
            Ok(())
        };
        match each() {
            Ok(()) => {
                println!("Published {name} {}: each device that runs it takes it.", &commit[..12]);
                released.insert(name.clone(), json!(commit));
            }
            Err(err) => {
                ok = false;
                println!("  couldn't send out {name}: {err:#}");
            }
        }
    }
    let _ = std::fs::write(cache().join("services-released"), serde_json::to_vec(&released).unwrap_or_default());
    ok
}
