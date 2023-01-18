use std::process::Command;

use anyhow::{bail, Result};

#[derive(Debug)]
pub(crate) struct Cmd {
    cmd: String,
    args: Vec<String>,
}

impl Cmd {
    pub(crate) fn new(input: String) -> Result<Self> {
        let parts = shell_words::split(&input)?;
        if let Some((cmd, args)) = parts.split_first() {
            return Ok(Self {
                cmd: cmd.clone(),
                args: args.to_vec(),
            });
        }

        bail!("Unable to parse requested subcommand after certificate fetch.")
    }

    pub(crate) fn execute(&self) -> Result<()> {
        Command::new(&self.cmd).args(&self.args).spawn()?.wait()?;

        Ok(())
    }
}
