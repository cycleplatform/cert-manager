use anyhow::{Context, Result};
use clap::Parser;
use env_logger::Env;
use std::{io::Write, path::PathBuf, thread::sleep, time::Duration};
use termcolor::{Color, ColorChoice, ColorSpec, StandardStream, WriteColor};

use crate::{cert::CertificateManager, config::Config};

mod api;
mod cert;
mod cmd;
mod config;

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

fn main() -> Result<()> {
    env_logger::Builder::from_env(Env::default().default_filter_or("info"))
        .format_timestamp_secs()
        .init();

    print_welcome_message();

    let cli = Cli::parse();
    let config = Config::new(cli.config.as_deref())?
        .merge_args(&cli)
        .validate()?;

    loop {
        log::info!("Fetching certificate for domain {}", config.domain);

        let cm = CertificateManager::new(&config);
        let res = cm.fetch_certificate();

        if let Err(err) = res {
            log::error!("Failed to fetch certificate: {:?}", err);
            log::info!("Retrying in 3 hours...");
            sleep(Duration::from_secs(3 * 60 * 60));
            continue;
        }

        log::info!("Successfully fetched certificate bundle.");

        let cert = res.unwrap();
        let duration = cert.duration_until_refetch(
            config
                .refresh_days
                .try_into()
                .with_context(|| "Failed to convert refresh days into i64")?,
        );

        let filename_override = config.filename.as_deref();

        if let Err(err) = cert
            .write_to_disk(&config.certificate_path, filename_override)
            .with_context(|| {
                format!(
                    "Failed to write files. \nCertificate: {}\nKey: {}",
                    cert.get_certificate_full_filepath(&config.certificate_path, filename_override),
                    cert.get_private_key_full_filepath(&config.certificate_path, filename_override)
                )
            })
        {
            log::error!("{:?}", err);
            log::info!("Retrying in 3 hours...");
            sleep(Duration::from_secs(3 * 60 * 60));
            continue;
        }

        log::info!(
            "Wrote certificate bundle to {}",
            cert.get_certificate_full_filepath(&config.certificate_path, filename_override)
        );

        log::info!(
            "Wrote private key to {}",
            cert.get_private_key_full_filepath(&config.certificate_path, filename_override)
        );

        if let Some(cmd) = config.exec.as_deref() {
            log::info!("Executing command '{}'", cmd);
            let c = cmd::Cmd::new(cmd.into())?;

            if let Err(err) = c.execute() {
                log::error!("Failed executing command: {:?}", err);
                log::info!("Retrying in 3 hours...");
                sleep(Duration::from_secs(3 * 60 * 60));
                continue;
            }
        }

        log::info!("Next fetch in {} days", duration.num_days());

        sleep(
            duration
                .to_std()
                .with_context(|| "Failed attempting to sleep.")?,
        )
    }
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
