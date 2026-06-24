//! Generate a Bitcoin Core styled user agent string
//! based on `bdk_floresta` and `floresta-wire` versions.

use std::fs;

use toml::Value;

fn main() {
    // Get bdk_floresta's version.
    let bdk_floresta_version = get_bdk_floresta_version();

    // Get floresta-wire's version from Cargo.toml or fallback to Cargo.lock
    let floresta_wire_version = source_dependency_version("floresta-wire").unwrap();

    // Build bdk_floresta's user agent in Bitcoin Core style:
    // `/floresta-wire:A.B.C/bdk-floresta:X.Y.Z`
    let user_agent = format!(
        "/floresta-wire:{}/bdk-floresta:{}/",
        floresta_wire_version,
        bdk_floresta_version.replace('v', ""),
    );

    println!("cargo:rustc-env=USER_AGENT={user_agent}");
    println!("cargo:rustc-env=GIT_DESCRIBE={bdk_floresta_version}");
    println!("cargo:rustc-env=FLORESTA_WIRE_VERSION={floresta_wire_version}");

    // Re-run if HEAD, build.rs, `Cargo.toml` or `Cargo.lock` change
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-changed=Cargo.lock");
}

/// Source `bdk_floresta`'s version from `Cargo.toml`.
fn get_bdk_floresta_version() -> String {
    let manifest = fs::read_to_string("Cargo.toml").unwrap();
    let toml: Value = toml::from_str(&manifest).unwrap();
    format!(
        "v{}",
        toml["package"]["version"]
            .as_str()
            .expect("Missing version in Cargo.toml")
    )
}

/// Try to get the version of a particular dependency.
///
/// Sources it from `Cargo.toml`, or falls back to `Cargo.lock`.
fn source_dependency_version(dep: &str) -> Option<String> {
    if let Some(version) = source_version_from_cargo_toml(dep) {
        return Some(version);
    }
    source_version_from_cargo_lock(dep)
}

/// Source the version of a dependency from `Cargo.toml`.
fn source_version_from_cargo_toml(dep_name: &str) -> Option<String> {
    let manifest = fs::read_to_string("Cargo.toml").ok()?;
    let toml: Value = toml::from_str(&manifest).ok()?;

    // Check all dependency sections
    for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
        if let Some(deps) = toml.get(section) {
            if let Some(dep) = deps.get(dep_name) {
                // Handle table format with git + rev
                if let Some(dep_table) = dep.as_table() {
                    // If it's a git dependency, extract the commit hash
                    if dep_table.contains_key("git") && dep_table.contains_key("rev") {
                        if let Some(rev) = dep_table.get("rev").and_then(|r| r.as_str()) {
                            // Use the short commit hash (first 7 hex chars)
                            return Some(format!("git-{}", &rev[..7.min(rev.len())]));
                        }
                    }
                    // Regular version field
                    if let Some(version) = dep_table.get("version").and_then(|v| v.as_str()) {
                        return Some(version.to_string());
                    }
                }
                // Handle string format: floresta-wire = "1.0.0"
                if let Some(version_str) = dep.as_str() {
                    return Some(version_str.to_string());
                }
            }
        }
    }

    None
}

/// Source the version of a dependency from `Cargo.lock`.
fn source_version_from_cargo_lock(dep_name: &str) -> Option<String> {
    let lock_content = std::fs::read_to_string("Cargo.lock").ok()?;
    let lock: toml::Value = toml::from_str(&lock_content).ok()?;

    // Cargo.lock has an array of packages.
    let packages = lock.get("package")?.as_array()?;

    for package in packages {
        if package.get("name")?.as_str()? == dep_name {
            // Check if it's a git dependency with a source field.
            if let Some(source) = package.get("source").and_then(|s| s.as_str()) {
                if source.starts_with("git+") {
                    // For git dependencies, try to extract the short commit
                    // hash. Format: git+https://github.com/...#d79e135f40515a859850dd59a6ee057f11c69128
                    if let Some(commit) = source.split('#').nth(1) {
                        // Use first 7 characters of commit hash.
                        return Some(format!("git-{}", &commit[..7.min(commit.len())]));
                    }
                }
            }
            // Regular versioned dependency.
            return Some(package.get("version")?.as_str()?.to_string());
        }
    }

    None
}
