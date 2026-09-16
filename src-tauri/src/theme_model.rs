//! Local theme data, including fields retained to render saved v1/v2 documents exactly.
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct StyleAdvice {
    pub accent: Option<String>,
    pub background: Option<String>,
    pub surface: Option<String>,
    pub sidebar: Option<String>,
    pub mood: Option<String>,
}
