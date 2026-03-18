use std::path::Path;

use serde::Deserialize;

use crate::runtime::AppHandle;
use crate::tex_doc;
use crate::types::{TexBlock, TexDocumentPayload};
use crate::CommandResult;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OpenTexDocumentArgs {
    pub(crate) file_path: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SaveTexDocumentArgs {
    pub(crate) file_path: String,
    pub(crate) blocks: Vec<TexBlock>,
}

pub(crate) fn open_document(
    _app: AppHandle,
    file_path: String,
) -> CommandResult<TexDocumentPayload> {
    tex_doc::open_tex_document(Path::new(&file_path))
}

pub(crate) fn save_document(
    _app: AppHandle,
    file_path: String,
    blocks: Vec<TexBlock>,
) -> CommandResult<TexDocumentPayload> {
    tex_doc::save_tex_document(Path::new(&file_path), &blocks)
}
