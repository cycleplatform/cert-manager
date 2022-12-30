use serde::Deserialize;


#[derive(Deserialize, Debug)]
pub struct ResponseEnvelope<T> {
    data: T
}

impl<T> ResponseEnvelope<T> {
    pub fn into_inner(self) -> T {
        self.data
    }
}