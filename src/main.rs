use anyhow::{Context, Result};
use clap::Parser;
use env_logger::Env;
use std::{io::Write, path::PathBuf, thread::sleep, time::Duration};
use termcolor::{Color, ColorChoice, ColorSpec, StandardStream, WriteColor};

use crate::{
    cert::{CertificateManager, CycleCert},
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
    #[arg(short, long)]
    apikey: Option<String>,

    /// The number of days before the expiration to refresh this certificate. Must be a positive number.
    #[arg(short, long, default_value = "14")]
    refresh_days: Option<u64>,

    /// The hub ID the DNS record is associated with.
    #[arg(long)]
    hub: Option<String>,

    /// A command to run after successfully fetching a certificate. Useful for
    /// restarting services that are dependent on the certificates.
    #[arg(short, long)]
    exec: Option<String>,
}

/// How long to wait before trying again after any failure.
const RETRY_DELAY: Duration = Duration::from_secs(3 * 60 * 60);

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

    let refresh_days: i64 = config
        .refresh_days
        .try_into()
        .with_context(|| "Failed to convert refresh days into i64")?;

    let exec = config
        .exec
        .as_deref()
        .map(|cmd| cmd::Cmd::new(cmd.into()))
        .transpose()
        .with_context(|| "Invalid exec command")?;

    let mut progress: Option<Progress> = None;

    loop {
        log::info!("Fetching certificate for domain {}", config.domain);

        let cert = match CertificateManager::new(&config).fetch_certificate() {
            Ok(cert) => cert,
            Err(err) => {
                log::error!("Failed to fetch certificate: {:?}", err);
                retry_later();
                continue;
            }
        };

        log::info!("Successfully fetched certificate bundle.");

        let current = match progress.take() {
            Some(p) if p.cert.same_material(&cert) => p,
            _ => Progress::new(cert),
        };
        let current = progress.insert(current);

        if current.is_complete(&config) {
            log::info!("Certificate unchanged since last fetch; nothing to do.");
        } else if let Err(err) = apply(&config, exec.as_ref(), current) {
            log::error!("{:?}", err);
            retry_later();
            continue;
        }

        let duration = current.cert.duration_until_refetch(refresh_days);

        log::info!("Next fetch in {} days", duration.num_days());

        sleep(
            duration
                .to_std()
                .with_context(|| "Failed attempting to sleep.")?,
        )
    }
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

fn retry_later() {
    log::info!("Retrying in 3 hours...");
    sleep(RETRY_DELAY);
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
