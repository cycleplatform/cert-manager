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
        let status = Command::new(&self.cmd).args(&self.args).status()?;

        if !status.success() {
            bail!("Command '{}' exited with {}", self.cmd, status);
        }

        Ok(())
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn test_execute_success() -> Result<()> {
        Cmd::new("true".into())?.execute()
    }

    #[test]
    fn test_execute_failure_status() -> Result<()> {
        let res = Cmd::new("false".into())?.execute();
        assert!(res.is_err(), "A non-zero exit status should be an error.");

        Ok(())
    }
}
