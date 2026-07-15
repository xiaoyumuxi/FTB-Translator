use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CmpTargetEdit {
    pub index: usize,
    pub target: String,
}
