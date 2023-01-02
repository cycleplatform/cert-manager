use anyhow::{Context, Result};
use clap::Parser;
use env_logger::Env;
use std::{io::Write, path::PathBuf, thread::sleep, time::Duration};
use termcolor::{Color, ColorChoice, ColorSpec, StandardStream, WriteColor};

use crate::{
    cert::{CertificateFetcher, CertificateManager},
    config::Config,
};

mod api;
mod cert;
mod config;

#[derive(Parser)]
#[command(author = "Petrichor, Inc.", version, about, long_about = None)]
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
    target: Option<String>,

    /// Overrides the filename of the certificate. By default, it will be the name of the domain the cert
    /// is applicable for.
    #[arg(short, long)]
    filename: Option<String>,

    /// The cluster the certificate is on. By default, it is the main api.cycle.io cluster.
    #[arg(long)]
    cluster: Option<String>,

    /// Your Cycle API Key. For more information, see https://docs.cycle.io/docs/hubs/API-access/api-key-generate
    #[arg(short, long)]
    api_key: Option<String>,

    /// The number of days before the expiration to refresh this certificate. Must be a positive number.
    #[arg(short, long, default_value = "14")]
    refresh_days: Option<u64>,
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
            log::info!("Retrying in 15 seconds...");
            sleep(Duration::from_secs(15));
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

        let filename_override = config.filename_override.as_deref();

        cert.write_to_disk(&config.certificate_path, filename_override)
            .with_context(|| {
                format!(
                    "Failed to write certificate to path {}/{}",
                    config.certificate_path,
                    cert.get_certificate_filename(filename_override)
                )
            })?;

        log::info!(
            "Wrote certificate bundle to {}/{}",
            config.certificate_path,
            cert.get_certificate_filename(filename_override)
        );

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
