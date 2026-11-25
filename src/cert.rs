use std::{
    fs::{create_dir_all, metadata, remove_file, rename, set_permissions, File},
    io::{self, Write},
};

use anyhow::{bail, Context};
use base64::{engine::general_purpose, Engine as _};
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

        // pass private to get the full certificate
        query.push(("private", "true"));

        let response = reqwest::blocking::Client::new()
            .get(format!(
                "https://{}/v1/dns/tls/certificates/lookup",
                self.config.core
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
    /// Whether `other` contains the same certificate and key as this one.
    pub(crate) fn same_material(&self, other: &CycleCert) -> bool {
        self.bundle.trim() == other.bundle.trim()
            && self.private_key.trim() == other.private_key.trim()
    }

    pub(crate) fn write_to_disk(&self, path: &str, filename: Option<&str>) -> io::Result<()> {
        create_dir_all(path)?;

        // Decode both up front so a bad payload doesn't leave a half-updated pair on disk.
        let cert_bytes = general_purpose::STANDARD
            .decode(self.bundle.trim())
            .map_err(to_io_error)?;

        let key_bytes = general_purpose::STANDARD
            .decode(self.private_key.trim())
            .map_err(to_io_error)?;

        // Key first: a reader that sees the new key with the old bundle fails loudly
        // on mismatch, and the window between the two renames is tiny.
        write_atomic(
            &self.get_private_key_full_filepath(path, filename),
            &key_bytes,
        )?;
        write_atomic(
            &self.get_certificate_full_filepath(path, filename),
            &cert_bytes,
        )?;

        Ok(())
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
        let remaining = date - Utc::now();
        let minimum = Duration::hours(3);

        if remaining < minimum {
            return minimum;
        }

        remaining
    }
}

/// Writes `bytes` to a temp file next to `path`, then renames it over `path`, so
/// readers never see a partially written file.
///
/// If `path` already exists, its permissions are copied to the new file so a
/// user's chmod survives renewals. If the rename fails (for example when `path`
/// is itself a bind mount, which returns EBUSY), it falls back to rewriting
/// `path` in place, which is how earlier versions behaved.
fn write_atomic(path: &str, bytes: &[u8]) -> io::Result<()> {
    let tmp = format!("{}.tmp", path);

    if let Err(err) = write_tmp(&tmp, path, bytes) {
        let _ = remove_file(&tmp);
        return Err(err);
    }

    if let Err(err) = rename(&tmp, path) {
        log::warn!(
            "Unable to atomically replace {} ({}); writing in place instead.",
            path,
            err
        );
        let _ = remove_file(&tmp);

        let mut file = File::create(path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }

    Ok(())
}

fn write_tmp(tmp: &str, path: &str, bytes: &[u8]) -> io::Result<()> {
    let mut file = File::create(tmp)?;
    file.write_all(bytes)?;
    file.sync_all()?;

    if let Ok(existing) = metadata(path) {
        set_permissions(tmp, existing.permissions())?;
    }

    Ok(())
}

fn to_io_error(err: base64::DecodeError) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("Base64 decode error: {}", err),
    )
}

#[cfg(test)]
mod tests {
    use base64::{engine::general_purpose, Engine as _};
    use chrono::{Datelike, NaiveDate, Timelike};
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn test_writing_bundle() -> anyhow::Result<()> {
        let dir = tempdir()?;

        let bundle_raw = "CONTENTS OF CERTIFICATE HERE";
        let private_key_raw = "Key to the castle";

        let bundle_b64 = general_purpose::STANDARD.encode(bundle_raw);
        let private_key_b64 = general_purpose::STANDARD.encode(private_key_raw);

        let cert = CycleCert {
            domains: vec!["cycle.io".to_string()],
            bundle: bundle_b64.clone(),
            private_key: private_key_b64.clone(),
            events: Events {
                generated: Utc::now(),
            },
        };

        cert.write_to_disk(dir.path().to_str().unwrap(), None)?;

        let bundle_file = std::fs::read_to_string(dir.path().join("cycle_io.ca-bundle"))?;
        assert_eq!(bundle_raw, bundle_file);

        let key_file = std::fs::read_to_string(dir.path().join("cycle_io.key"))?;
        assert_eq!(private_key_raw, key_file);

        Ok(())
    }

    #[test]
    fn test_writing_bundle_multiple_domains() -> anyhow::Result<()> {
        let dir = tempdir()?;

        let bundle_raw = "CONTENTS OF CERTIFICATE HERE";
        let private_key_raw = "Key to the castle";

        let bundle_b64 = general_purpose::STANDARD.encode(bundle_raw);
        let private_key_b64 = general_purpose::STANDARD.encode(private_key_raw);

        let cert = CycleCert {
            domains: vec!["cycle.io".to_string(), "petrichor.io".to_string()],
            bundle: bundle_b64.clone(),
            private_key: private_key_b64.clone(),
            events: Events {
                generated: Utc::now(),
            },
        };

        cert.write_to_disk(dir.path().to_str().unwrap(), None)?;

        let bundle_file =
            std::fs::read_to_string(dir.path().join("cycle_io_petrichor_io.ca-bundle"))?;
        assert_eq!(bundle_raw, bundle_file);

        let key_file = std::fs::read_to_string(dir.path().join("cycle_io_petrichor_io.key"))?;
        assert_eq!(private_key_raw, key_file);

        Ok(())
    }

    fn test_cert(bundle: &str, key: &str) -> CycleCert {
        CycleCert {
            domains: vec!["cycle.io".to_string()],
            bundle: general_purpose::STANDARD.encode(bundle),
            private_key: general_purpose::STANDARD.encode(key),
            events: Events {
                generated: Utc::now(),
            },
        }
    }

    #[test]
    fn test_overwrite_leaves_no_tmp_files() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let path = dir.path().to_str().unwrap();

        test_cert("old bundle", "old key").write_to_disk(path, None)?;
        test_cert("new bundle", "new key").write_to_disk(path, None)?;

        assert_eq!(
            std::fs::read_to_string(dir.path().join("cycle_io.ca-bundle"))?,
            "new bundle"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("cycle_io.key"))?,
            "new key"
        );

        let mut names: Vec<_> = std::fs::read_dir(dir.path())?
            .map(|e| e.map(|e| e.file_name().into_string().unwrap()))
            .collect::<Result<_, _>>()?;
        names.sort();
        assert_eq!(names, vec!["cycle_io.ca-bundle", "cycle_io.key"]);

        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn test_overwrite_preserves_permissions() -> anyhow::Result<()> {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempdir()?;
        let path = dir.path().to_str().unwrap();
        let key_path = dir.path().join("cycle_io.key");

        test_cert("old bundle", "old key").write_to_disk(path, None)?;
        std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600))?;

        test_cert("new bundle", "new key").write_to_disk(path, None)?;

        let mode = std::fs::metadata(&key_path)?.permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);

        Ok(())
    }

    #[test]
    fn test_same_material() {
        let a = test_cert("bundle", "key");

        assert!(a.same_material(&test_cert("bundle", "key")));
        assert!(!a.same_material(&test_cert("bundle", "other key")));
        assert!(!a.same_material(&test_cert("other bundle", "key")));
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
            bundle: general_purpose::STANDARD.encode("dummy"),
            private_key: general_purpose::STANDARD.encode("dummy-key"),
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
                should_be_num_days - 1
            } else {
                should_be_num_days
            }
        );

        Ok(())
    }
}
