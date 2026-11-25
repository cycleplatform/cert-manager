use std::{
    fmt,
    fs::{create_dir_all, metadata, remove_file, rename, set_permissions, File},
    io::{self, Write},
};

use anyhow::{anyhow, Context};
use base64::{engine::general_purpose, Engine as _};
use chrono::{DateTime, Duration, Utc};
use reqwest::StatusCode;
use serde::Deserialize;
use x509_parser::pem::Pem;

use crate::api::ApiResult;

/// Used only if the expiry can't be read from the certificate itself: the
/// number of days from generation that a Cycle certificate expires.
const FALLBACK_EXPIRATION_DAYS: i64 = 90;

/// The Cycle API returned an error response.
#[derive(Debug)]
pub(crate) struct ApiRejection {
    pub status: StatusCode,
    pub message: String,
}

impl ApiRejection {
    /// Whether the API rejected our credentials (API key or hub), as opposed to
    /// something that may resolve on its own, like a certificate not existing yet.
    pub(crate) fn is_auth_error(&self) -> bool {
        self.status == StatusCode::UNAUTHORIZED || self.status == StatusCode::FORBIDDEN
    }
}

impl fmt::Display for ApiRejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Cycle API returned {}: {}", self.status, self.message)
    }
}

impl std::error::Error for ApiRejection {}

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

        let status = response.status();

        match response.json::<ApiResult<CycleCert>>() {
            Ok(ApiResult::Ok(c)) => Ok(c.data),
            Ok(ApiResult::Err(err)) => Err(ApiRejection {
                status,
                message: format!(
                    "{} ({})...{}",
                    err.error.title,
                    err.error.code,
                    err.error.detail.unwrap_or_default()
                ),
            }
            .into()),
            Err(err) if !status.is_success() => Err(ApiRejection {
                status,
                message: format!("unreadable error response: {}", err),
            }
            .into()),
            Err(err) => Err(anyhow!(err).context("Failed parsing response from Cycle API.")),
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

    /// When this certificate expires. Read from the bundle (the earliest
    /// `notAfter` of any certificate in it, since the chain is only valid until
    /// then), falling back to an estimate from the generation date.
    pub(crate) fn expires_at(&self) -> DateTime<Utc> {
        match self.parse_not_after() {
            Ok(date) => date,
            Err(err) => {
                log::warn!(
                    "Unable to read expiry from certificate, estimating from generation date: {:?}",
                    err
                );
                self.events.generated + Duration::days(FALLBACK_EXPIRATION_DAYS)
            }
        }
    }

    fn parse_not_after(&self) -> anyhow::Result<DateTime<Utc>> {
        let bytes = general_purpose::STANDARD.decode(self.bundle.trim())?;
        let mut earliest: Option<DateTime<Utc>> = None;

        for pem in Pem::iter_from_buffer(&bytes) {
            let pem = pem.map_err(|e| anyhow!("Invalid PEM block: {}", e))?;
            let cert = pem
                .parse_x509()
                .map_err(|e| anyhow!("Invalid certificate: {}", e))?;
            let timestamp = cert.validity().not_after.timestamp();
            let not_after = DateTime::from_timestamp(timestamp, 0)
                .with_context(|| format!("Expiry timestamp {} out of range", timestamp))?;

            earliest = match earliest {
                Some(current) if current <= not_after => Some(current),
                _ => Some(not_after),
            };
        }

        earliest.context("No certificates found in bundle")
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
    use base64::engine::general_purpose;
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

    fn bundle_cert(pem: &str, generated: DateTime<Utc>) -> CycleCert {
        CycleCert {
            domains: vec!["cycle.io".to_string()],
            bundle: general_purpose::STANDARD.encode(pem),
            private_key: general_purpose::STANDARD.encode("key"),
            events: Events { generated },
        }
    }

    // The fixtures hold two self-signed certs; the one expiring first has
    // notAfter = 2026-12-25T14:43:06Z.
    const EARLIEST_EXPIRY: i64 = 1_798_209_786;

    #[test]
    fn test_expires_at_reads_earliest_not_after() {
        let expected = DateTime::from_timestamp(EARLIEST_EXPIRY, 0).unwrap();

        for pem in [
            include_str!("../tests/fixtures/bundle.pem"),
            include_str!("../tests/fixtures/bundle-reversed.pem"),
        ] {
            assert_eq!(bundle_cert(pem, Utc::now()).expires_at(), expected);
        }
    }

    #[test]
    fn test_expires_at_falls_back_to_generated() {
        let generated = Utc::now() - Duration::days(10);
        let cert = bundle_cert("not a certificate", generated);

        assert_eq!(
            cert.expires_at(),
            generated + Duration::days(FALLBACK_EXPIRATION_DAYS)
        );
    }

    #[test]
    fn test_auth_errors() {
        let rejection = |status| ApiRejection {
            status,
            message: String::new(),
        };

        assert!(rejection(StatusCode::UNAUTHORIZED).is_auth_error());
        assert!(rejection(StatusCode::FORBIDDEN).is_auth_error());
        assert!(!rejection(StatusCode::NOT_FOUND).is_auth_error());
        assert!(!rejection(StatusCode::INTERNAL_SERVER_ERROR).is_auth_error());
    }
}
