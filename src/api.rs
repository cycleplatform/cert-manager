use serde::Deserialize;

#[derive(Debug,  Deserialize)]
pub struct DataEnvelope<T> {
    pub data: T,
}

#[derive(Debug, Deserialize)]
pub struct ErrorEnvelope {
    pub error: ApiError,
}

/// The details of an error response
#[derive(Debug, Deserialize)]
pub struct ApiError {
    pub details: String,
    pub code: usize,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum ApiResult<T> {
    Ok(DataEnvelope<T>),
    Err(ErrorEnvelope),
}
