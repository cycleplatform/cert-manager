use std::{
    fs::{create_dir_all, File},
    io::{self, Write},
};

use anyhow::{Context, bail};
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;

use crate::api::ApiResult;

/// The number of days from generation that a certificate expires.
const EXPIRATION_DAYS: i64 = 90;

pub(crate) trait CertificateFetcher {
    fn fetch_certificate(&self) -> anyhow::Result<CycleCert>;
}

pub(crate) struct CertificateManager<'a> {
    config: &'a super::config::Config,
}

impl<'a> CertificateManager<'a> {
    pub fn new(config: &'a super::config::Config) -> Self {
        Self { config }
    }
}

impl<'a> CertificateFetcher for CertificateManager<'a> {
    fn fetch_certificate(&self) -> anyhow::Result<CycleCert> {
        let response = reqwest::blocking::Client::new()
            .get(format!(
                "https://{}/v1/dns/certificates",
                self.config.cluster
            ))
            .header("X-API-KEY", self.config.apikey.as_str())
            .query(&[("domain", self.config.domain.as_str())])
            .send()?;

        let cert = response.json::<ApiResult<CycleCert>>()
            .with_context(|| "Failed parsing response from Cycle API.")?;

        match cert {
            ApiResult::Ok(c) => Ok(c.data),
            ApiResult::Err(err) => bail!(format!("{} - {}", err.error.title, err.error.detail)),
        }
    }
}

#[derive(Deserialize, Debug)]
pub(crate) struct Events {
    generated: DateTime<Utc>,
}

#[derive(Deserialize, Debug)]
pub(crate) struct CycleCert {
    domains: Vec<String>,
    bundle: String,
    events: Events,
}

impl CycleCert {
    pub(crate) fn write_to_disk(&self, path: &str, filename: Option<&str>) -> io::Result<()> {
        create_dir_all(path)?;
        let mut output = File::create(format!("{}/{}", path, self.get_certificate_filename(filename)))?;
        output.write_all(self.bundle.as_bytes())
    }

    pub(crate) fn get_certificate_filename(&self, filename: Option<&str>) -> String {
        let name = if let Some(n) = filename {
            n.to_owned()
        } else {
            self.domains.join("_")
        };
        format!("{}.ca-bundle", name)
    }

    pub(crate) fn duration_until_refetch(&self, refetch_days: i64) -> Duration {
        let date = (self.events.generated + Duration::days(EXPIRATION_DAYS))
            - Duration::days(refetch_days);
        Utc::now() - date
    }
}
