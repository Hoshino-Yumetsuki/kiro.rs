/// Build script: derives GIT_VERSION at compile time.
/// - If the current commit has a tag → use the tag (e.g. `v1.1.30`)
/// - Otherwise → use the short commit hash (e.g. `a1b2c3d`)
fn main() {
    let version = git_version();
    println!("cargo:rustc-env=GIT_VERSION={version}");
}

fn git_version() -> String {
    if let Ok(tag) = run_git(&["describe", "--tags", "--exact-match"]) {
        return tag;
    }
    if let Ok(hash) = run_git(&["rev-parse", "--short", "HEAD"]) {
        return hash;
    }
    env!("CARGO_PKG_VERSION").to_string()
}

fn run_git(args: &[&str]) -> Result<String, ()> {
    let output = std::process::Command::new("git")
        .args(args)
        .output()
        .map_err(|_| ())?;
    if output.status.success() {
        let s = String::from_utf8(output.stdout).map_err(|_| ())?;
        let trimmed = s.trim().to_string();
        if !trimmed.is_empty() {
            return Ok(trimmed);
        }
    }
    Err(())
}
