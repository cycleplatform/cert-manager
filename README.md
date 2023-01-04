# Cycle Cert Bot

![workflow](https://github.com/cycleplatform/cert-manager/actions/workflows/rust.yml/badge.svg)

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

This will download the certificate bundle and install it in the current working directory with the name `<YOUR DOMAIN>.ca-bundle`. 

_Note - If your certificate applies to multiple domains, they will be separated by an underscore. All periods are also replaced with underscores. Therefore, if your domain were e.g. cycle.io, the bundle would be saved to a file `cycle_io.ca-bundle`. If your domains were `cycle.io` and `test.com`, the bundle would be saved to `cycle_io_test_com.ca-bundle`_

The process will sleep in the background, until 14 days before the certificate expires, when it will attempt to fetch the latest certificate again. (Cycle renews certificates 65 days after generation).

If the bot fails to fetch the certificate for any reason, it will wait 3 hours and make the request again, indefinitely. Verify that your setup is correct before running this process in the background.


## Configuration

While all configuration options can be set via command line, it may be preferrable to use a config file. By default, Cycle Certificate Manager loads a `config.toml` from the current working directory.

### Options

| Option | Required | Description |
| ------ | -------- | ----------- |
| domain | true | The hostname of the desired certificate |
| apikey | true |Your Cycle API Key. For more information, see https://docs.cycle.io/docs/hubs/API-access/api-key-generate |
| hub | true |The ID of the hub the desired certificate belongs to |
| refresh_days | false| The number of days before the expiration to refresh this certificate. Must be a positive number. |
| certificate_path | false | The path to write the fetched certificate bundle to. If none is selected, it will be written to the current directory. |
| filename | false | Overrides the filename of the certificate. By default, it will be the name of the domain the cert is applicable for |
| cluster | false | The cluster the certificate is on. By default, it is the main api.cycle.io cluster |


### Example

**config.toml**

```toml
domain = "myapp.mysite.com"
refresh_days = 5
apikey = "<YOUR API KEY>"
```
