use std::{
    fs::{create_dir_all, File},
    io::{Result, Write},
};

use serde::Deserialize;

#[derive(Deserialize, Debug)]
pub struct CycleCert {
    domains: Vec<String>,
    private_key: String,
    bundle: String,
    csr: String,
    issuer_certificate: String,
    hub_id: Option<String>,
}

impl CycleCert {
    pub fn write_to_disk(&self, path: &str) -> Result<()> {
        create_dir_all(path)?;
        let mut output = File::create(format!("{}/{}", path, self.get_certificate_filename()))?;
        output.write_all(self.bundle.as_bytes())
    }

    pub fn get_certificate_filename(&self) -> String {
        format!("{}.ca-bundle", self.domains.join("_"))
    }
}
