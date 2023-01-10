use std::{
    fs::{create_dir_all, File},
    io::{self, Write},
};

use anyhow::{bail, Context};
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;

use crate::api::ApiResult;

/// The number of days from generation that a certificate expires.
const EXPIRATION_DAYS: i64 = 90;

pub(crate) struct CertificateManager<'a> {
    config: &'a super::config::Config,
}

impl<'a> CertificateManager<'a> {
    pub fn new(config: &'a super::config::Config) -> Self {
        Self { config }
    }

    pub fn fetch_certificate(&self) -> anyhow::Result<CycleCert> {
        let mut query = vec![("domain", self.config.domain.as_str())];

        if self.config.wildcard {
            query.push(("wildcard", "true"));
        }

        let response = reqwest::blocking::Client::new()
            .get(format!(
                "https://{}/v1/dns/tls/certificates/lookup",
                self.config.cluster
            ))
            .header(
                "Authorization",
                format!("Bearer {}", self.config.apikey.as_str()),
            )
            .header("X-HUB-ID", self.config.hub.as_str())
            .query(&query[..])
            .send()?;

        let cert = response
            .json::<ApiResult<CycleCert>>()
            .with_context(|| "Failed parsing response from Cycle API.")?;

        match cert {
            ApiResult::Ok(c) => Ok(c.data),
            ApiResult::Err(err) => bail!(format!(
                "{}...{}",
                err.error.title,
                err.error.detail.unwrap_or_default()
            )),
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
    private_key: String,
}

impl CycleCert {
    pub(crate) fn write_to_disk(&self, path: &str, filename: Option<&str>) -> io::Result<()> {
        create_dir_all(path)?;
        let mut file = File::create(self.get_certificate_full_filepath(path, filename))?;
        file.write_all(self.bundle.as_bytes())?;

        // Reuse the file var for writing the key
        file = File::create(self.get_private_key_full_filepath(path, filename))?;
        file.write_all(self.private_key.as_bytes())
    }

    pub(crate) fn get_certificate_full_filepath(
        &self,
        path: &str,
        filename: Option<&str>,
    ) -> String {
        let name = if let Some(n) = filename {
            n.to_owned()
        } else {
            self.domains.join("_").replace('.', "_")
        };
        format!("{}/{}.ca-bundle", path, name)
    }

    pub(crate) fn get_private_key_full_filepath(
        &self,
        path: &str,
        filename: Option<&str>,
    ) -> String {
        let name = if let Some(n) = filename {
            n.to_owned()
        } else {
            self.domains.join("_").replace('.', "_")
        };
        format!("{}/{}.key", path, name)
    }

    pub(crate) fn duration_until_refetch(&self, refetch_days: i64) -> Duration {
        let date = (self.events.generated + Duration::days(EXPIRATION_DAYS))
            - Duration::days(refetch_days);
        date - Utc::now()
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Datelike, NaiveDate, Timelike};
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn test_writing_bundle() -> anyhow::Result<()> {
        let dir = tempdir()?;

        let content = String::from("CONTENTS OF CERTIFICATE HERE");

        let cert = CycleCert {
            domains: vec!["cycle.io".to_string()],
            bundle: content.clone(),
            private_key: "Key to the castle".into(),
            events: Events {
                generated: Utc::now(),
            },
        };

        cert.write_to_disk(dir.path().to_str().unwrap(), None)?;

        let cert_file_content = std::fs::read_to_string(dir.path().join("cycle_io.ca-bundle"))?;
        assert_eq!(content, cert_file_content);

        Ok(())
    }

    #[test]
    fn test_writing_bundle_multiple_domains() -> anyhow::Result<()> {
        let dir = tempdir()?;

        let content = String::from("CONTENTS OF CERTIFICATE HERE");

        let cert = CycleCert {
            domains: vec!["cycle.io".to_string(), "petrichor.io".to_string()],
            bundle: content.clone(),
            private_key: "Key to the castle".into(),
            events: Events {
                generated: Utc::now(),
            },
        };

        cert.write_to_disk(dir.path().to_str().unwrap(), None)?;

        let cert_file_content =
            std::fs::read_to_string(dir.path().join("cycle_io_petrichor_io.ca-bundle"))?;
        assert_eq!(content, cert_file_content);

        Ok(())
    }

    #[test]
    fn test_calculate_expiration_time() -> anyhow::Result<()> {
        let generated_prior_days = 2;
        let days_before_refresh = 14;
        let now = Utc::now();
        let start_of_day = NaiveDate::from_ymd_opt(now.year(), now.month(), now.day())
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap();

        let cert = CycleCert {
            domains: vec!["cycle.io".to_string(), "petrichor.io".to_string()],
            bundle: String::from("CONTENTS OF CERTIFICATE HERE"),
            private_key: "Key to the castle".into(),
            events: Events {
                generated: DateTime::<Utc>::from_utc(start_of_day, Utc)
                    - Duration::days(generated_prior_days),
            },
        };

        let dur_from_now = cert.duration_until_refetch(days_before_refresh);

        let should_be_num_days = EXPIRATION_DAYS - days_before_refresh - generated_prior_days;
        assert_eq!(
            dur_from_now.num_days(),
            if now.hour() > 0 {
                // not a whole day if we're not at exactly midnight
                should_be_num_days - 1
            } else {
                should_be_num_days
            }
        );

        Ok(())
    }
}
