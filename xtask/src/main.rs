//! Build helper for OpenBBS.
//!
//! `cargo xtask build`        bump the patch version, stamp CHANGELOG.md, release build
//! `cargo xtask bump <part>`  bump patch, minor or major without building
//! `cargo xtask version`      print the current version
//!
//! The version lives in `[workspace.package]` in the root `Cargo.toml`.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = workspace_root();
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["build", rest @ ..] => {
            let version = bump(&root, Part::Patch)?;
            println!("Building OpenBBS v{version}");
            let status = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
                .current_dir(&root)
                .args(["build", "--release", "--workspace", "--exclude", "xtask"])
                .args(rest)
                .status()
                .context("running cargo build")?;
            if !status.success() {
                bail!("cargo build failed (version was already bumped to v{version})");
            }
            Ok(())
        }
        ["bump", part] => {
            let part = match *part {
                "patch" => Part::Patch,
                "minor" => Part::Minor,
                "major" => Part::Major,
                other => bail!("unknown version part `{other}`; use patch, minor or major"),
            };
            let version = bump(&root, part)?;
            println!("Bumped to v{version}");
            Ok(())
        }
        ["version"] => {
            println!("{}", read_version(&root)?.0);
            Ok(())
        }
        _ => {
            eprintln!("usage: cargo xtask <build [cargo args]|bump patch|minor|major|version>");
            std::process::exit(2);
        }
    }
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask lives inside the workspace")
        .to_path_buf()
}

#[derive(Clone, Copy)]
enum Part {
    Patch,
    Minor,
    Major,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Version {
    major: u64,
    minor: u64,
    patch: u64,
}

impl Version {
    fn parse(s: &str) -> Result<Self> {
        let parts: Vec<&str> = s.split('.').collect();
        let [major, minor, patch] = parts.as_slice() else {
            bail!("version `{s}` is not MAJOR.MINOR.PATCH");
        };
        Ok(Self {
            major: major.parse()?,
            minor: minor.parse()?,
            patch: patch.parse()?,
        })
    }

    fn bumped(self, part: Part) -> Self {
        match part {
            Part::Patch => Self {
                patch: self.patch + 1,
                ..self
            },
            Part::Minor => Self {
                minor: self.minor + 1,
                patch: 0,
                ..self
            },
            Part::Major => Self {
                major: self.major + 1,
                minor: 0,
                patch: 0,
            },
        }
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// Find `version = "x.y.z"` inside `[workspace.package]` and return it with its line index.
fn find_version(manifest: &str) -> Result<(Version, usize)> {
    let mut in_section = false;
    for (i, line) in manifest.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_section = trimmed == "[workspace.package]";
            continue;
        }
        if !in_section {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("version") {
            let value = rest.trim_start().strip_prefix('=').map(str::trim);
            if let Some(v) = value.and_then(|v| v.strip_prefix('"')?.strip_suffix('"')) {
                return Ok((Version::parse(v)?, i));
            }
        }
    }
    bail!("no version in [workspace.package] of Cargo.toml")
}

fn read_version(root: &Path) -> Result<(Version, usize, String)> {
    let manifest = fs::read_to_string(root.join("Cargo.toml"))?;
    let (v, line) = find_version(&manifest)?;
    Ok((v, line, manifest))
}

fn bump(root: &Path, part: Part) -> Result<Version> {
    let (old, line, manifest) = read_version(root)?;
    let new = old.bumped(part);
    let updated: Vec<String> = manifest
        .lines()
        .enumerate()
        .map(|(i, l)| {
            if i == line {
                l.replacen(&format!("\"{old}\""), &format!("\"{new}\""), 1)
            } else {
                l.to_string()
            }
        })
        .collect();
    fs::write(root.join("Cargo.toml"), updated.join("\n") + "\n")?;

    let changelog_path = root.join("CHANGELOG.md");
    let changelog = fs::read_to_string(&changelog_path).context("reading CHANGELOG.md")?;
    fs::write(&changelog_path, stamp_changelog(&changelog, new, &today()))?;

    // Keep Cargo.lock in step so the new version is recorded.
    let _ = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .current_dir(root)
        .args(["update", "--workspace", "--offline", "--quiet"])
        .status();
    Ok(new)
}

/// Turn the `## [Unreleased]` heading into a heading for `version` and add a
/// fresh, empty Unreleased section above it. If Unreleased has no notes, a
/// "Build only" note is recorded so every version still has an entry.
fn stamp_changelog(changelog: &str, version: Version, date: &str) -> String {
    const HEADING: &str = "## [Unreleased]";
    let fresh = format!("{HEADING}\n\n## [{version}] - {date}");
    let Some(start) = changelog.find(HEADING) else {
        return format!("{changelog}\n{fresh}\n\n- Build only, no notable changes.\n");
    };
    let after = &changelog[start + HEADING.len()..];
    let section_end = after.find("\n## ").unwrap_or(after.len());
    let notes = after[..section_end].trim();
    let body = if notes.is_empty() {
        "\n\n- Build only, no notable changes.\n".to_string()
    } else {
        format!("\n\n{notes}\n")
    };
    let rest = &after[section_end..];
    format!("{}{fresh}{body}{rest}", &changelog[..start])
}

/// Today's date (UTC) as YYYY-MM-DD, without pulling in a date crate.
fn today() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = (secs / 86_400) as i64;
    // Civil-from-days, Howard Hinnant's algorithm.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bumps() {
        let v = Version::parse("0.0.9").unwrap();
        assert_eq!(v.bumped(Part::Patch).to_string(), "0.0.10");
        assert_eq!(v.bumped(Part::Minor).to_string(), "0.1.0");
        assert_eq!(v.bumped(Part::Major).to_string(), "1.0.0");
    }

    #[test]
    fn finds_workspace_version_only() {
        let manifest = "[package]\nversion = \"9.9.9\"\n\n[workspace.package]\n# note\nversion = \"0.0.3\"\nedition = \"2024\"\n";
        let (v, line) = find_version(manifest).unwrap();
        assert_eq!(v.to_string(), "0.0.3");
        assert_eq!(line, 5);
    }

    #[test]
    fn stamps_unreleased_notes() {
        let log = "# Changelog\n\n## [Unreleased]\n\n### Added\n- Thing\n\n## [0.0.1] - 2026-01-01\n\n- Old\n";
        let out = stamp_changelog(log, Version::parse("0.0.2").unwrap(), "2026-02-02");
        assert_eq!(
            out,
            "# Changelog\n\n## [Unreleased]\n\n## [0.0.2] - 2026-02-02\n\n### Added\n- Thing\n\n## [0.0.1] - 2026-01-01\n\n- Old\n"
        );
    }

    #[test]
    fn stamps_empty_unreleased() {
        let log = "# Changelog\n\n## [Unreleased]\n\n## [0.0.1] - 2026-01-01\n\n- Old\n";
        let out = stamp_changelog(log, Version::parse("0.0.2").unwrap(), "2026-02-02");
        assert!(out.contains(
            "## [0.0.2] - 2026-02-02\n\n- Build only, no notable changes.\n\n## [0.0.1]"
        ));
    }

    #[test]
    fn date_is_iso() {
        let d = today();
        assert_eq!(d.len(), 10);
        assert_eq!(&d[4..5], "-");
    }
}
