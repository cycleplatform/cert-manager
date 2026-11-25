# Cycle Cert Bot

![](https://github.com/cycleplatform/cert-manager/actions/workflows/rust.yml/badge.svg)
![](https://img.shields.io/crates/v/cycle-certs)

https://cycle.io

A bot for fetching and keeping TLS certificates generated via Cycle's DNS
service up to date.

## Installation

This binary can be installed one of several different ways:

### Cargo

If you're using cargo:

`cargo install cycle-certs`

### Download

Go to the releases section and download the binary for your system, then copy it into a folder in your path.
If your system is not listed, try compiling with

### Docker

The following command will run the process in the background. Remove the `-d`
argument to see the output first and verify that it works as expected.

`docker run -dit --name cycle-certs -v $(pwd):/certs cycleplatform/cycle-certs`

By default, the process will look for the config file in the mounted volume
(`/certs/config.toml` inside the container). You can provide your own location
by passing the `--config=<FILENAME>` option instead.

To keep your API key out of the config file (and out of the volume other
containers read certificates from), pass it as an environment variable instead:

`docker run -dit --name cycle-certs -e CYCLE_CERTS_APIKEY=<API KEY> -v $(pwd):/certs cycleplatform/cycle-certs`

Images are published for `linux/amd64` and `linux/arm64`, tagged with each
release version (e.g. `1.4.0`, `1.4`, `1`) and `latest`.

### From Source

This assumes you have set up a rust toolchain.

Clone this repo and run this in the root:

`cargo build --release`

Then copy the bin to a location in your path.

#### Linux

`mv ./target/release/cycle-certs /usr/local/bin`

## Quick Start

To run straight from the command line, run

`cycle-certs --domain=<YOUR DOMAIN> --apikey=<API KEY> --hub=<HUB ID>`

This will download a certificate bundle and private key associated with the specified domain. By default, the files will be stored in the current working directory with the names:

-   `<YOUR DOMAIN>.ca-bundle`
-   `<YOUR DOMAIN>.key`

_Note - If your certificate applies to multiple domains, they will be separated by an underscore. All periods are also replaced with underscores. Therefore, if your domain were e.g. cycle.io, the bundle would be saved to a file `cycle_io.ca-bundle`. If your domains were `cycle.io` and `test.com`, the bundle would be saved to `cycle_io_test_com.ca-bundle`. To avoid this, pass the `--filename` argument._

The process then checks Cycle for a renewed certificate every 24 hours (configurable with `check_interval_hours`), and only writes files when the certificate has changed. Cycle renews certificates 65 days after generation, so a renewed certificate is picked up within a day.

If the API rejects your API key or hub ID on startup, the process exits with an error. Any other failure is retried every 3 hours (or every `check_interval_hours`, if shorter), indefinitely. If Cycle hasn't renewed the certificate and it's within 20 days of expiring, a warning is logged on every check, escalating to an error within 7 days.

Files are replaced atomically (written to a temporary file, then renamed into place), so other processes never read a partially written certificate. Any permissions you set on existing files are kept.

The `exec` command and webhooks only run when a new certificate has been written. If one of them fails (including the `exec` command exiting with a non-zero status), the bot retries from the step that failed on the next attempt, without re-running the steps that already succeeded.

Each process manages a single certificate. To manage several, run one instance per domain.

## Configuration

While all configuration options can be set via command line, it may be preferrable to use a config file. By default, Cycle Certificate Manager loads a `config.toml` from the current working directory.

### Options

| Option           | Required | Description                                                                                                            |
| ---------------- | -------- | ---------------------------------------------------------------------------------------------------------------------- |
| domain           | true     | The hostname of the desired certificate                                                                                |
| apikey           | true     | Your Cycle API Key. Can also be set with the `CYCLE_CERTS_APIKEY` environment variable. For more information, see https://docs.cycle.io/docs/hubs/API-access/api-key-generate |
| hub              | true     | The ID of the hub the desired certificate belongs to                                                                   |
| check_interval_hours | false | How often to check Cycle for a renewed certificate, in hours. Defaults to 24.                                         |
| refresh_days     | false    | Deprecated and ignored. Replaced by `check_interval_hours`.                                                            |
| certificate_path | false    | The path to write the fetched certificate bundle to. If none is selected, it will be written to the current directory. |
| filename         | false    | Overrides the filename of the certificate. By default, it will be the name of the domain the cert is applicable for    |
| core             | false    | The core the certificate is on. By default, it is the main api.cycle.io core                                           |

### Example

**config.toml**

```toml
domain = "myapp.mysite.com"
apikey = "<YOUR API KEY>"
hub = "<YOUR HUB ID>"
```
