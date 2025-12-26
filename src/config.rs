use serde::{Deserialize, Serialize};
use std::path::PathBuf;
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct SharedBufferConfiguration {
    pub path: PathBuf,
    pub max_publishers: usize,
    pub max_subscribers: usize,
}
