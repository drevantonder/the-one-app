//! An app's service, built and published for every platform.

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

/// Each service's program last published from here, and the commit it was built from.
pub(super) fn services_released() -> serde_json::Map<String, Value> {
    platform::files::read_json(&cache().join("services-released"))
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
            let built = deploy::service::build_service(&src.join("apps").join(name), &commit[..12], PLATFORMS.iter(), &out)?.context("it has no service")?;
            for (platform, program) in built.made {
                let version = Version { commit: commit.into(), target: platform.target.into(), committed };
                publish::publish_latest(&crate::node(), key, &[(oned_update::latest::Thing::service_name(name, &version.target), Some(program), version, "-".into())]).with_context(|| format!("publish it for {}", platform.target))?;
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
    platform::files::write_json(&cache().join("services-released"), &released);
    ok
}
