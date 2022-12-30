use anyhow::{bail, Context, Result};
use clap::Parser;
use env_logger::Env;
use std::{path::PathBuf, thread::sleep, time::Duration};

use crate::config::Config;

mod api;
mod cert;
mod config;

#[derive(Parser)]
#[command(author = "Petrichor, Inc.", version, about, long_about = None)]
pub(crate) struct Cli {
    /// Sets a custom config file
    #[arg(short, long, value_name = "FILE")]
    config: Option<PathBuf>,

    /// The hostname of the record you wish to fetch. If zone and record are set,
    /// they take priority.
    #[arg(short, long)]
    domain: Option<String>,

    /// The ID of the zone on Cycle containing the record for the certificate you wish to monitor. Takes priority over domain.
    #[arg(short, long)]
    zone: Option<String>,

    /// The ID of the record on Cycle for the certificate you wish to monitor. Takes priority over domain.
    #[arg(short, long)]
    record: Option<String>,

    /// The ID of the hub containing the certificate you wish to monitor. If one is not provided here, it
    /// must be provided in the config file.
    #[arg(long)]
    hub: Option<String>,

    /// The path to write the fetched certificate bundle to. If none is selected, it will be written to the
    /// current directory.
    #[arg(short, long)]
    target: Option<String>,
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();

    let cli = Cli::parse();
    let config = Config::new(cli.config.as_deref())?.parse_args(&cli)?;

    loop {
        log::info!("Fetching certificate...");

        let resp = reqwest::blocking::get("https://api-local.dev.cycle.io/v1/dns/certs")?
            .json::<api::ResponseEnvelope<cert::CycleCert>>()
            .with_context(|| "Failed to fetch certificate bundle")?;

        log::info!("Successfully fetched certificate bundle");

        let duration = Duration::from_secs(config.refresh_days * 24 * 60 * 60);
        let cert = resp.into_inner();

        cert.write_to_disk(&config.certificate_path)
            .with_context(|| {
                format!(
                    "Failed to write certificate to path {}",
                    config.certificate_path
                )
            })?;

        log::info!(
            "Wrote certificate bundle to {}/{}",
            config.certificate_path,
            cert.get_certificate_filename()
        );

        log::info!("Next fetch in {} days", duration.as_secs() / 60 / 60 / 24);

        sleep(duration)
    }
}
