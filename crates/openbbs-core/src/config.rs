//! The `openbbs.toml` configuration file.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub bbs: BbsConfig,
    pub ssh: SshConfig,
    pub paths: PathsConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct BbsConfig {
    /// Name shown to callers.
    pub name: String,
    /// The sysop's handle.
    pub sysop: String,
    /// Most callers allowed online at once.
    pub max_nodes: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct SshConfig {
    /// Address to listen on, for example `0.0.0.0:2222`.
    pub listen: String,
    /// Where the server's host key is stored (created on first run).
    pub host_key: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct PathsConfig {
    /// Folder for the database, host key and other runtime data.
    pub data: PathBuf,
    /// Folder for menus and ANSI screens.
    pub themes: PathBuf,
    /// Folder that holds file-area uploads.
    pub files: PathBuf,
    /// Folder for door programs and their drop files.
    pub doors: PathBuf,
}

impl Default for BbsConfig {
    fn default() -> Self {
        Self {
            name: "My OpenBBS".into(),
            sysop: "Sysop".into(),
            max_nodes: 8,
        }
    }
}

impl Default for SshConfig {
    fn default() -> Self {
        Self {
            listen: "0.0.0.0:2222".into(),
            host_key: PathBuf::from("data/ssh_host_ed25519_key"),
        }
    }
}

impl Default for PathsConfig {
    fn default() -> Self {
        Self {
            data: PathBuf::from("data"),
            themes: PathBuf::from("themes"),
            files: PathBuf::from("data/files"),
            doors: PathBuf::from("doors"),
        }
    }
}

impl Config {
    /// Load and validate a config file.
    pub fn load(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path)
            .with_context(|| format!("reading config file {}", path.display()))?;
        let config: Config =
            toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        config.validate()?;
        Ok(config)
    }

    /// Write this config to `path` as TOML.
    pub fn save(&self, path: &Path) -> Result<()> {
        let text = toml::to_string_pretty(self).context("serializing config")?;
        fs::write(path, text).with_context(|| format!("writing {}", path.display()))
    }

    /// Check values that serde alone can't.
    pub fn validate(&self) -> Result<()> {
        if self.bbs.name.trim().is_empty() {
            bail!("bbs.name must not be empty");
        }
        if self.bbs.max_nodes == 0 {
            bail!("bbs.max_nodes must be at least 1");
        }
        self.ssh
            .listen
            .parse::<std::net::SocketAddr>()
            .with_context(|| format!("ssh.listen is not a valid address: {}", self.ssh.listen))?;
        Ok(())
    }

    /// Path of the SQLite database inside the data folder.
    pub fn database_path(&self) -> PathBuf {
        self.paths.data.join("openbbs.db")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_round_trips_through_toml() {
        let config = Config::default();
        let text = toml::to_string_pretty(&config).unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(config, back);
        back.validate().unwrap();
    }

    #[test]
    fn partial_file_uses_defaults() {
        let config: Config = toml::from_str("[bbs]\nname = \"The Pit\"\n").unwrap();
        assert_eq!(config.bbs.name, "The Pit");
        assert_eq!(config.ssh.listen, "0.0.0.0:2222");
    }

    #[test]
    fn rejects_bad_listen_address() {
        let mut config = Config::default();
        config.ssh.listen = "not an address".into();
        assert!(config.validate().is_err());
    }

    #[test]
    fn rejects_unknown_keys() {
        assert!(toml::from_str::<Config>("[bbs]\nnmae = \"typo\"\n").is_err());
    }
}
