use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct Config {
    pub input_node_id: Option<u32>,
    pub input_node_name: Option<String>,
    pub sample_rate: u32,
    pub channels: u32,
    pub output_file: Option<String>,
}

impl Config {
    pub fn path() -> PathBuf {
        if let Some(dir) = dirs::config_dir() {
            let mut p = dir;
            p.push("scrivener");
            let _ = std::fs::create_dir_all(&p);
            p.push("config.toml");
            p
        } else {
            PathBuf::from("config.toml")
        }
    }

    pub fn load() -> Result<Self> {
        let path = Self::path();
        if path.exists() {
            let s = std::fs::read_to_string(&path).context("read config")?;
            let cfg: Config = toml::from_str(&s).context("parse config")?;
            Ok(cfg)
        } else {
            Ok(Config {
                sample_rate: 48000,
                channels: 1,
                ..Default::default()
            })
        }
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::path();
        let s = toml::to_string_pretty(self).context("serialize config")?;
        std::fs::write(path, s).context("write config")
    }
}
