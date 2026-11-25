use anyhow::{Context, Result};
use chrono::Utc;
use clap::Parser;
use env_logger::Env;
use std::{io::Write, path::PathBuf, thread::sleep, time::Duration};
use termcolor::{Color, ColorChoice, ColorSpec, StandardStream, WriteColor};

use crate::{
    cert::{ApiRejection, CertificateManager, CycleCert},
    config::Config,
};

mod api;
mod cert;
mod cmd;
mod config;
mod webhook;

#[derive(Parser)]
#[command(author, about, long_about = None)]
pub(crate) struct Cli {
    /// Sets a custom config file
    #[arg(short, long, value_name = "FILE")]
    config: Option<PathBuf>,

    /// The hostname of the desired certificate.
    #[arg(short, long)]
    domain: Option<String>,

    /// The path to write the fetched certificate bundle to. If none is selected, it will be written to the
    /// current directory.
    #[arg(short, long)]
    path: Option<String>,

    /// Overrides the filename of the certificate. By default, it will be the name of the domain the cert
    /// is applicable for.
    #[arg(short, long)]
    filename: Option<String>,

    /// The core the certificate is on. By default, it is the main api.cycle.io core.
    #[arg(long)]
    core: Option<String>,

    /// Your Cycle API Key. For more information, see https://docs.cycle.io/docs/hubs/API-access/api-key-generate
    #[arg(short, long, env = "CYCLE_CERTS_APIKEY", hide_env_values = true)]
    apikey: Option<String>,

    /// How often to check Cycle for a renewed certificate, in hours. Defaults to 24.
    #[arg(long)]
    check_interval_hours: Option<u64>,

    /// Deprecated and ignored. Certificates are now checked every --check-interval-hours.
    #[arg(short, long, hide = true)]
    refresh_days: Option<u64>,

    /// The hub ID the DNS record is associated with.
    #[arg(long)]
    hub: Option<String>,

    /// A command to run after successfully fetching a certificate. Useful for
    /// restarting services that are dependent on the certificates.
    #[arg(short, long)]
    exec: Option<String>,
}

/// How long to wait before trying again after any failure (or the check
/// interval, if that's shorter).
const RETRY_DELAY: Duration = Duration::from_secs(3 * 60 * 60);

/// Log a warning when the latest certificate expires within this many days.
/// Cycle renews certificates with 25 days left, so this means renewal is late.
const EXPIRY_WARNING_DAYS: i64 = 20;

/// Log an error when the latest certificate expires within this many days.
const EXPIRY_ERROR_DAYS: i64 = 7;

/// Tracks which steps have completed for the most recently fetched certificate,
/// so a failure only retries the step that failed, and an unchanged certificate
/// doesn't rewrite files or fire hooks again.
struct Progress {
    cert: CycleCert,
    written: bool,
    exec_done: bool,
    webhooks_done: usize,
}

impl Progress {
    fn new(cert: CycleCert) -> Self {
        Self {
            cert,
            written: false,
            exec_done: false,
            webhooks_done: 0,
        }
    }

    fn is_complete(&self, config: &Config) -> bool {
        self.written && self.exec_done && self.webhooks_done >= config.webhooks.len()
    }
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(Env::default().default_filter_or("info"))
        .format_timestamp_secs()
        .init();

    print_welcome_message();

    // When running as PID 1 (e.g. in the scratch Docker image), the kernel won't apply
    // the default SIGTERM handler, so `docker stop` would hang until SIGKILL.
    ctrlc::set_handler(|| {
        log::info!("Received shutdown signal, exiting.");
        std::process::exit(0);
    })
    .with_context(|| "Failed to install signal handler")?;

    let cli = Cli::parse();
    let config = Config::new(cli.config.as_deref())?
        .merge_args(&cli)
        .validate()?;

    if config.refresh_days.is_some() {
        log::warn!(
            "refresh_days is deprecated and ignored. Certificates are checked every {} hours (check_interval_hours).",
            config.check_interval_hours
        );
    }

    let exec = config
        .exec
        .as_deref()
        .map(|cmd| cmd::Cmd::new(cmd.into()))
        .transpose()
        .with_context(|| "Invalid exec command")?;

    let interval = Duration::from_secs(config.check_interval_hours * 60 * 60);
    let retry_delay = RETRY_DELAY.min(interval);
    let mut progress: Option<Progress> = None;

    loop {
        let succeeded = check(&config, exec.as_ref(), &mut progress)?;

        if let Some(p) = progress.as_ref() {
            warn_if_expiring(&config.domain, &p.cert);
        }

        let delay = if succeeded { interval } else { retry_delay };

        log::info!("Next check in {}", format_duration(delay));
        sleep(delay);
    }
}

/// Fetches the certificate and applies it if it changed. Returns whether
/// everything succeeded. Only returns an error for failures that retrying
/// can't fix.
fn check(
    config: &Config,
    exec: Option<&cmd::Cmd>,
    progress: &mut Option<Progress>,
) -> Result<bool> {
    log::info!("Fetching certificate for domain {}", config.domain);

    let cert = match CertificateManager::new(config).fetch_certificate() {
        Ok(cert) => cert,
        Err(err) => {
            let is_auth_error = err
                .downcast_ref::<ApiRejection>()
                .is_some_and(ApiRejection::is_auth_error);

            // Fail fast on bad credentials at startup so a misconfiguration is
            // obvious, rather than buried in a log that only updates every few hours.
            if is_auth_error && progress.is_none() {
                return Err(err.context(
                    "The Cycle API rejected the request. Check your API key, hub ID, and the API key's permissions.",
                ));
            }

            log::error!("Failed to fetch certificate: {:?}", err);
            return Ok(false);
        }
    };

    log::info!("Successfully fetched certificate bundle.");

    let current = match progress.take() {
        Some(p) if p.cert.same_material(&cert) => p,
        _ => Progress::new(cert),
    };
    let current = progress.insert(current);

    if current.is_complete(config) {
        log::info!("Certificate unchanged since last check; nothing to do.");
        return Ok(true);
    }

    if let Err(err) = apply(config, exec, current) {
        log::error!("{:?}", err);
        return Ok(false);
    }

    Ok(true)
}

/// Runs whichever steps haven't completed yet for this certificate: writing
/// files, the exec command, then each webhook in order.
fn apply(config: &Config, exec: Option<&cmd::Cmd>, progress: &mut Progress) -> Result<()> {
    let path = config.certificate_path.as_str();
    let filename = config.filename.as_deref();

    if !progress.written {
        let cert_path = progress.cert.get_certificate_full_filepath(path, filename);
        let key_path = progress.cert.get_private_key_full_filepath(path, filename);

        progress
            .cert
            .write_to_disk(path, filename)
            .with_context(|| {
                format!(
                    "Failed to write files. \nCertificate: {}\nKey: {}",
                    cert_path, key_path
                )
            })?;

        log::info!("Wrote certificate bundle to {}", cert_path);
        log::info!("Wrote private key to {}", key_path);

        progress.written = true;
    }

    if !progress.exec_done {
        if let Some(cmd) = exec {
            log::info!(
                "Executing command '{}'",
                config.exec.as_deref().unwrap_or_default()
            );
            cmd.execute().with_context(|| "Failed executing command")?;
        }

        progress.exec_done = true;
    }

    webhook::call_remaining(&config.webhooks, &mut progress.webhooks_done)?;

    Ok(())
}

fn warn_if_expiring(domain: &str, cert: &CycleCert) {
    let expires = cert.expires_at();
    let remaining = expires - Utc::now();

    if remaining <= chrono::Duration::zero() {
        log::error!(
            "The latest certificate for {} expired at {}. Cycle has not renewed it.",
            domain,
            expires
        );
        return;
    }

    if remaining <= chrono::Duration::days(EXPIRY_ERROR_DAYS) {
        log::error!(
            "The latest certificate for {} expires in {} days ({}). Cycle has not renewed it.",
            domain,
            remaining.num_days(),
            expires
        );
        return;
    }

    if remaining <= chrono::Duration::days(EXPIRY_WARNING_DAYS) {
        log::warn!(
            "The latest certificate for {} expires in {} days ({}). Cycle normally renews it with 25 days left.",
            domain,
            remaining.num_days(),
            expires
        );
    }
}

fn format_duration(duration: Duration) -> String {
    let hours = duration.as_secs() / 3600;

    if hours == 1 {
        return "1 hour".to_owned();
    }

    format!("{} hours", hours)
}

fn print_welcome_message() {
    let mut stdout = StandardStream::stdout(ColorChoice::Always);
    stdout
        .set_color(ColorSpec::new().set_fg(Some(Color::Rgb(42, 167, 255))))
        .unwrap();
    writeln!(
        &mut stdout,
        "Cycle Certificate Manager v{}",
        env!("CARGO_PKG_VERSION")
    )
    .unwrap();

    stdout.reset().unwrap();
    writeln!(&mut stdout).unwrap();
}
