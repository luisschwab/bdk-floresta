//! Generate a Bitcoin Core styled user agent
//! string based on `bdk_floresta` and `floresta-wire` versions.

fn main() {
    // Get `bdk_floresta` version.
    let version = get_version_from_git()
        .unwrap_or_else(|| get_version_from_manifest().unwrap());

    // Get `floresta-wire` version from Cargo.lock or Cargo.toml.
    let floresta_wire_version = get_dependency_version("floresta-wire")
        .unwrap_or_else(|| "unknown".to_string());

    // Build `bdk_floresta`'s user agent in Bitcoin Core style:
    // `/floresta-wire:A.B.C/bdk-floresta:X.Y.Z`.
    let user_agent = format!(
        "/floresta-wire:{}/bdk-floresta:{}/",
        floresta_wire_version,
        version.replace("v", ""),
    );

    println!("cargo:rustc-env=USER_AGENT={}", user_agent);
    println!("cargo:rustc-env=GIT_DESCRIBE={}", version);
    println!(
        "cargo:rustc-env=FLORESTA_WIRE_VERSION={}",
        floresta_wire_version
    );

    // Re-run if the build script or git HEAD changes.
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-changed=Cargo.lock");
}

fn get_version_from_manifest() -> Result<String, std::io::Error> {
    let manifest = std::fs::read_to_string("Cargo.toml")?;
    let toml: toml::Value = toml::from_str(&manifest).unwrap();
    Ok(format!("v{}", toml["package"]["version"].as_str().unwrap()))
}

fn get_version_from_git() -> Option<String> {
    std::process::Command::new("git")
        .args(["describe", "--tags", "--always", "--dirty"])
        .output()
        .map(|output| {
            if !output.status.success() {
                return None;
            }
            let mut git_description = String::from_utf8(output.stdout).unwrap();
            // Remove the trailing `\n`.
            git_description.pop();

            // If tags are not pulled, git will return the short commit ID.
            if !git_description.starts_with("v") {
                return None;
            }
            Some(git_description)
        })
        .ok()?
}

fn get_dependency_version(dep_name: &str) -> Option<String> {
    // Try Cargo.lock first...
    if let Some(version) = get_version_from_cargo_lock(dep_name) {
        return Some(version);
    }

    // ..or fallback to Cargo.toml.
    get_version_from_cargo_toml(dep_name)
}

fn get_version_from_cargo_lock(dep_name: &str) -> Option<String> {
    let lock_content = std::fs::read_to_string("Cargo.lock").ok()?;
    let lock: toml::Value = toml::from_str(&lock_content).ok()?;

    // Cargo.lock has an array of packages.
    let packages = lock.get("package")?.as_array()?;

    for package in packages {
        if package.get("name")?.as_str()? == dep_name {
            // Check if it's a git dependency with a source field.
            if let Some(source) = package.get("source").and_then(|s| s.as_str())
            {
                if source.starts_with("git+") {
                    // For git dependencies, try to extract the short commit
                    // hash. Format: git+https://github.com/...#d79e135f40515a859850dd59a6ee057f11c69128
                    if let Some(commit) = source.split('#').nth(1) {
                        // Use first 7 characters of commit hash.
                        return Some(format!(
                            "git-{}",
                            &commit[..7.min(commit.len())]
                        ));
                    }
                }
            }
            // Regular versioned dependency.
            return Some(package.get("version")?.as_str()?.to_string());
        }
    }

    None
}

fn get_version_from_cargo_toml(dep_name: &str) -> Option<String> {
    let manifest = std::fs::read_to_string("Cargo.toml").ok()?;
    let toml: toml::Value = toml::from_str(&manifest).ok()?;

    // Check dependencies section.
    if let Some(deps) = toml.get("dependencies") {
        if let Some(dep) = deps.get(dep_name) {
            // Handle both string and table formats:
            // floresta-wire = "1.0.0"
            // floresta-wire = { version = "1.0.0", ... }
            if let Some(version_str) = dep.as_str() {
                return Some(version_str.to_string());
            } else if let Some(version) = dep.get("version") {
                return version.as_str().map(|s| s.to_string());
            }
        }
    }

    None
}
