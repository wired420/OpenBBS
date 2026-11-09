//! Theme files (menus, screens and colors), read from the theme folder with
//! the built-in default theme as a fallback.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use openbbs_core::cp437;
use serde::Deserialize;

/// The default theme, compiled into the binary.
const DEFAULT_THEME: &[(&str, &[u8])] = &[
    ("theme.toml", include_bytes!("../theme-default/theme.toml")),
    (
        "menus/main.toml",
        include_bytes!("../theme-default/menus/main.toml"),
    ),
    (
        "menus/sysop.toml",
        include_bytes!("../theme-default/menus/sysop.toml"),
    ),
    (
        "screens/welcome.ans",
        include_bytes!("../theme-default/screens/welcome.ans"),
    ),
    (
        "screens/welcome-info.txt",
        include_bytes!("../theme-default/screens/welcome-info.txt"),
    ),
    (
        "screens/newuser.txt",
        include_bytes!("../theme-default/screens/newuser.txt"),
    ),
    (
        "screens/goodbye.txt",
        include_bytes!("../theme-default/screens/goodbye.txt"),
    ),
    (
        "screens/comingsoon.txt",
        include_bytes!("../theme-default/screens/comingsoon.txt"),
    ),
];

/// Colors used when OpenBBS draws a menu itself. Each is a pipe code string.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Colors {
    pub frame: String,
    pub title: String,
    pub subtitle: String,
    pub key: String,
    pub bracket: String,
    pub label: String,
    pub highlight: String,
    pub hint: String,
}

impl Default for Colors {
    fn default() -> Self {
        Self {
            frame: "|09".into(),
            title: "|15".into(),
            subtitle: "|03".into(),
            key: "|11".into(),
            bracket: "|08".into(),
            label: "|07".into(),
            highlight: "|15|17".into(),
            hint: "|08".into(),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct ThemeFile {
    colors: Colors,
}

#[derive(Debug, Clone)]
pub struct Theme {
    dir: PathBuf,
    pub colors: Colors,
}

impl Theme {
    /// Load the theme in `dir`. Missing files fall back to the default theme.
    pub fn load(dir: &Path) -> Result<Self> {
        let mut theme = Self {
            dir: dir.to_path_buf(),
            colors: Colors::default(),
        };
        if let Some(bytes) = theme.read("theme.toml")? {
            let text = String::from_utf8_lossy(&bytes);
            let file: ThemeFile = toml::from_str(&text).context("parsing theme.toml")?;
            theme.colors = file.colors;
        }
        Ok(theme)
    }

    /// Read a theme file, from disk if present, else from the built-in theme.
    pub fn read(&self, rel: &str) -> Result<Option<Vec<u8>>> {
        let path = self.dir.join(rel);
        match fs::read(&path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(DEFAULT_THEME
                .iter()
                .find(|(name, _)| *name == rel)
                .map(|(_, bytes)| bytes.to_vec())),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    /// Read a screen by name (`welcome` finds `screens/welcome.ans`, then
    /// `.txt`), decoded to UTF-8. CP437 art is converted automatically.
    pub fn screen(&self, name: &str) -> Result<Option<String>> {
        if !is_safe_name(name) {
            return Ok(None);
        }
        for ext in ["ans", "txt"] {
            if let Some(bytes) = self.read(&format!("screens/{name}.{ext}"))? {
                return Ok(Some(cp437::decode_screen(&bytes)));
            }
        }
        Ok(None)
    }

    /// Read a menu definition's text by name.
    pub fn menu_source(&self, name: &str) -> Result<Option<String>> {
        if !is_safe_name(name) {
            return Ok(None);
        }
        Ok(self
            .read(&format!("menus/{name}.toml"))?
            .map(|b| String::from_utf8_lossy(&b).into_owned()))
    }
}

/// Theme file names may not reach outside the theme folder.
fn is_safe_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Copy the built-in theme into `dir`, keeping any files that already exist.
/// Returns how many files were written.
pub fn install_default(dir: &Path) -> Result<usize> {
    let mut written = 0;
    for (rel, bytes) in DEFAULT_THEME {
        let path = dir.join(rel);
        if path.exists() {
            continue;
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, bytes).with_context(|| format!("writing {}", path.display()))?;
        written += 1;
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn falls_back_to_built_in_theme() {
        let theme = Theme::load(Path::new("/nonexistent/theme")).unwrap();
        let welcome = theme.screen("welcome").unwrap().unwrap();
        assert!(welcome.contains('█'), "CP437 art should be converted");
        assert!(theme.menu_source("main").unwrap().is_some());
        assert!(theme.screen("../etc/passwd").unwrap().is_none());
        assert_eq!(theme.colors.key, "|11");
    }

    #[test]
    fn disk_files_override_built_in() {
        let dir = std::env::temp_dir().join(format!("openbbs-theme-{}", std::process::id()));
        install_default(&dir).unwrap();
        fs::write(dir.join("screens/goodbye.txt"), "custom bye").unwrap();
        let theme = Theme::load(&dir).unwrap();
        assert_eq!(theme.screen("goodbye").unwrap().unwrap(), "custom bye");
        assert_eq!(install_default(&dir).unwrap(), 0);
        let _ = fs::remove_dir_all(&dir);
    }
}
