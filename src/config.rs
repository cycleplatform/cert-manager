use std::{
    path::{Path, PathBuf},
    str::FromStr,
};

use anyhow::{bail, Result};
use serde::Deserialize;

use crate::Cli;

#[derive(Deserialize)]
pub struct Config {
    pub domain: String,
    pub refresh_days: u64,
    pub certificate_path: String,
    pub filename_override: Option<String>,
    pub cluster: String,
    pub apikey: String,
}

impl Config {
    pub fn new(source: Option<&Path>) -> Result<Self> {
        let default_path = PathBuf::from_str("./config")?;
        let file = source.map(|p| p.to_path_buf()).unwrap_or(default_path);

        let c = config::Config::builder()
            .add_source(
                config::File::with_name(
                    file.to_str()
                        .ok_or_else(|| anyhow::anyhow!("Unable to format path to string"))?,
                )
                .required(false),
            )
            .set_default("certificate_path", "./")?
            .set_default("refresh_days", 14)?
            .set_default("cluster", "api.cycle.io")?
            // Allow the user to pass these items via CLI params. We validate later.
            .set_default("domain", "")?
            .set_default("apikey", "")?
            .build()?;

        Ok(c.try_deserialize()?)
    }

    /// Merge the CLI options with the config file.
    pub(crate) fn merge_args(mut self, cli: &Cli) -> Self {
        if let Some(certificate_path) = cli.target.as_deref() {
            self.certificate_path = certificate_path.to_owned();
        }

        if let Some(domain) = cli.domain.as_deref() {
            self.domain = domain.to_owned();
        }

        if let Some(refresh_days) = cli.refresh_days {
            self.refresh_days = refresh_days;
        }

        if let Some(filename_override) = cli.filename.as_deref() {
            self.filename_override = Some(filename_override.to_owned());
        }

        if let Some(cluster) = cli.cluster.as_deref() {
            self.cluster = cluster.to_owned();
        }

        if let Some(key) = cli.api_key.as_deref() {
            self.apikey = key.to_owned();
        }

        self
    }

    pub(crate) fn validate(self) -> Result<Self> {
        if self.domain.is_empty() {
            bail!("No hostname or DNS record provided to fetch certificate for");
        }

        if self.apikey.is_empty() {
            bail!("Missing API key - An API key must be provided.")
        }

        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[test]
    fn test_merged_config() -> Result<()> {
        let mut cfg = Config::new(None)?;
        let cli = crate::Cli::parse_from([
            "cycle-certs",
            "--domain=cycle.io",
            "--api-key=123",
            "--target=./certs",
            "--filename=certs",
            "--cluster=api.dev.cycle.io",
        ]);
        cfg = cfg.merge_args(&cli);

        assert_eq!(cfg.apikey, "123");
        assert_eq!(cfg.domain, "cycle.io");
        assert_eq!(cfg.certificate_path, "./certs");
        assert_eq!(cfg.filename_override, Some("certs".into()));
        assert_eq!(cfg.cluster, "api.dev.cycle.io");

        Ok(())
    }

    #[test]
    fn test_config_validation() -> Result<()> {
        let cfg = Config::new(None)?;
        let cli = crate::Cli::parse_from([""]);

        assert!(
            cfg.merge_args(&cli).validate().is_err(),
            "Config should fail to validate if apikey or domain aren't set."
        );

        Ok(())
    }
}
