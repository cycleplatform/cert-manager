use anyhow::{Context, Result};
use clap::Parser;
use env_logger::Env;
use std::{io::Write, path::PathBuf, thread::sleep, time::Duration};
use termcolor::{Color, ColorChoice, ColorSpec, StandardStream, WriteColor};

use crate::{api::ApiResult, config::Config};

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

    /// Overrides the filename of the certificate. By default, it will be the name of the domain the cert
    /// is applicable for.
    #[arg(short, long)]
    filename: Option<String>,

    /// The cluster the certificate is on. By default, it is the main api.cycle.io cluster.
    #[arg(long)]
    cluster: Option<String>,
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();

    print_welcome_message();

    let cli = Cli::parse();
    let config = Config::new(cli.config.as_deref())?
        .merge_args(&cli)
        .validate()?;

    loop {
        if let Some(d) = config.domain.as_deref() {
            log::info!("Fetching certificate for domain {}", d);
        } else if let Some(r) = config.record.as_ref() {
            log::info!("Fetching certificate for record {} in zone {}", r.record_id, r.zone_id);
        }

        let resp = reqwest::blocking::get(format!("https://{}/v1/dns/certs", config.cluster))?
            .json::<ApiResult<cert::CycleCert>>()
            .with_context(|| "Failed to fetch certificate bundle")?;

        log::info!("Successfully fetched certificate bundle");

        let duration = Duration::from_secs(config.refresh_days * 24 * 60 * 60);

        let cert = match resp {
            ApiResult::Ok(r) => r.data,
            ApiResult::Err(e) => anyhow::bail!("Failed to fetch certificate bundle: {:#?}", e),
        };

        cert.write_to_disk(&config.certificate_path)
            .with_context(|| {
                format!(
                    "Failed to write certificate to path {}/{}",
                    config.certificate_path,
                    cert.get_certificate_filename()
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

fn print_welcome_message() {
    let mut stdout = StandardStream::stdout(ColorChoice::Always);
    stdout
        .set_color(ColorSpec::new().set_fg(Some(Color::Rgb(42, 167, 255))))
        .unwrap();
    writeln!(
        &mut stdout,
        "Cycle Certificate Manager v{}\n",
        env!("CARGO_PKG_VERSION")
    )
    .unwrap();
}
