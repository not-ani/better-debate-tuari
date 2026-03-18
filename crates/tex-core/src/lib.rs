mod tex_doc;

use serde::{Deserialize, Serialize};

pub type CommandResult<T> = Result<T, String>;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TexTextRun {
    pub text: String,
    #[serde(default)]
    pub bold: bool,
    #[serde(default)]
    pub italic: bool,
    #[serde(default)]
    pub underline: bool,
    #[serde(default)]
    pub small_caps: bool,
    #[serde(default)]
    pub highlight_color: Option<String>,
    #[serde(default)]
    pub style_id: Option<String>,
    #[serde(default)]
    pub style_name: Option<String>,
    #[serde(default)]
    pub is_f8_cite: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TexBlock {
    pub id: String,
    pub kind: String,
    pub text: String,
    pub runs: Vec<TexTextRun>,
    pub level: Option<i64>,
    pub style_id: Option<String>,
    pub style_name: Option<String>,
    #[serde(default)]
    pub is_f8_cite: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TexDocumentPayload {
    pub file_path: String,
    pub file_name: String,
    pub paragraph_count: i64,
    pub blocks: Vec<TexBlock>,
}

pub use tex_doc::{open_tex_document, save_tex_document};
