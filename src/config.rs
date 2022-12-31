use std::{
    path::{Path, PathBuf},
    str::FromStr,
};

use anyhow::{bail, Result};
use serde::Deserialize;

use crate::Cli;

#[derive(Deserialize)]
pub struct RecordSettings {
    pub zone_id: String,
    pub record_id: String,
}

#[derive(Deserialize)]
pub struct Config {
    pub domain: Option<String>,
    pub record: Option<RecordSettings>,
    pub hub_id: String,
    pub refresh_days: u64,
    pub certificate_path: String,
    pub filename_override: Option<String>,
    pub cluster: String,
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
            .set_default("refresh_days", 30)?
            .set_default("hub_id", "")?
            .set_default("cluster", "api.cycle.io")?
            .build()?;

        Ok(c.try_deserialize()?)
    }

    /// Merge the CLI options with the config file. If there are any
    /// issues, returns an error.
    pub(crate) fn merge_args(mut self, cli: &Cli) -> Self {
        if let Some(hub_id) = cli.hub.as_deref() {
            self.hub_id = hub_id.to_owned();
        }

        if let Some(certificate_path) = cli.target.as_deref() {
            self.certificate_path = certificate_path.to_owned();
        }

        if let Some(hub_id) = cli.hub.as_deref() {
            self.hub_id = hub_id.to_owned();
        }

        if let Some(domain) = cli.domain.as_deref() {
            self.domain = Some(domain.to_owned());
        }

        if let (Some(record), Some(zone)) = (cli.record.as_deref(), cli.zone.as_deref()) {
            self.record = Some(RecordSettings {
                zone_id: zone.to_owned(),
                record_id: record.to_owned(),
            })
        }

        if let Some(filename_override) = cli.filename.as_deref() {
            self.filename_override = Some(filename_override.to_owned());
        }

        if let Some(cluster) = cli.cluster.as_deref() {
            self.cluster = cluster.to_owned();
        }

        self
    }

    pub(crate) fn validate(self) -> Result<Self> {
        if self.domain.is_none() && self.record.is_none() {
            bail!("No hostname or DNS record provided to fetch certificate for");
        }

        if self.hub_id.is_empty() {
            bail!("No hub ID provided in config file or arguments.");
        };

        if self.domain.is_some() && self.record.is_some() {
            log::warn!("Both domain and zone/record values are set. Defaulting to domain.");
        }

        Ok(self)
    }
}
