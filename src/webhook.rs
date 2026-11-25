use std::time::Duration;

use anyhow::{Context, Result};
use reqwest::{blocking::Client, Method};

use crate::config::Webhook;

/// Calls each webhook in order, starting at `*done`. `*done` is advanced after
/// each success, so a later call resumes at the webhook that failed instead of
/// calling the ones that already succeeded again.
pub(crate) fn call_remaining(webhooks: &[Webhook], done: &mut usize) -> Result<()> {
    if *done >= webhooks.len() {
        return Ok(());
    }

    let client = Client::new();

    for (i, hook) in webhooks.iter().enumerate().skip(*done) {
        log::info!("Calling webhook {} of {}", i + 1, webhooks.len());
        call(&client, hook).with_context(|| format!("Webhook {} failed", i + 1))?;
        *done = i + 1;
    }

    Ok(())
}

fn call(client: &Client, hook: &Webhook) -> Result<()> {
    let method = Method::from_bytes(hook.method.to_uppercase().as_bytes())?;

    let mut request = client
        .request(method, &hook.url)
        .timeout(Duration::from_secs(30));

    for (name, value) in &hook.headers {
        request = request.header(name, value);
    }

    if let Some(body) = &hook.body {
        request = request.body(body.clone());
    }

    request.send()?.error_for_status()?;

    Ok(())
}
