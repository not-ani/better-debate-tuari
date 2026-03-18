use std::collections::HashMap;
use std::fs::File;
use std::path::Path;

use roxmltree::Node;
use zip::ZipArchive;

use crate::docx_parse::{
    attribute_value, detect_heading_level, extract_paragraph_text, has_tag, is_f8_cite_style,
    read_style_map, read_zip_file, run_has_active_underline, run_has_property, run_highlight_class,
    run_style_id, run_style_name,
};
use crate::types::{TexBlock, TexDocumentPayload, TexTextRun};
use crate::util::{is_probable_author_line, path_display};
use crate::CommandResult;

fn paragraph_style_id(paragraph: Node<'_, '_>) -> Option<String> {
    let paragraph_props = paragraph.children().find(|node| has_tag(*node, "pPr"))?;
    let style_node = paragraph_props
        .children()
        .find(|node| has_tag(*node, "pStyle"))?;
    let style_id = attribute_value(style_node, "val")?;
    Some(style_id.to_string())
}

fn run_text(run: Node<'_, '_>) -> String {
    extract_paragraph_text(run)
}

fn paragraph_runs(paragraph: Node<'_, '_>, style_map: &HashMap<String, String>) -> Vec<TexTextRun> {
    let mut runs = Vec::new();

    let mut push_run = |run: Node<'_, '_>| {
        let text = run_text(run);
        if text.is_empty() {
            return;
        }
        let style_id = run_style_id(run);
        let style_name = run_style_name(run, style_map);
        let is_f8_cite = style_id.as_deref().map(is_f8_cite_style).unwrap_or(false)
            || style_name.as_deref().map(is_f8_cite_style).unwrap_or(false);
        runs.push(TexTextRun {
            text,
            bold: run_has_property(run, "b"),
            italic: run_has_property(run, "i"),
            underline: run_has_active_underline(run),
            small_caps: run_has_property(run, "smallCaps") || run_has_property(run, "caps"),
            highlight_color: run_highlight_class(run).map(str::to_string),
            style_id,
            style_name,
            is_f8_cite,
        });
    };

    for child in paragraph.children() {
        if has_tag(child, "r") {
            push_run(child);
            continue;
        }

        if has_tag(child, "hyperlink") {
            for run in child.children().filter(|node| has_tag(*node, "r")) {
                push_run(run);
            }
        }
    }

    if runs.is_empty() {
        let text = extract_paragraph_text(paragraph);
        if !text.is_empty() {
            runs.push(TexTextRun {
                text,
                bold: false,
                italic: false,
                underline: false,
                small_caps: false,
                highlight_color: None,
                style_id: None,
                style_name: None,
                is_f8_cite: false,
            });
        }
    }

    runs
}

fn build_tex_block(
    paragraph: Node<'_, '_>,
    order: usize,
    style_map: &HashMap<String, String>,
) -> TexBlock {
    let text = extract_paragraph_text(paragraph);
    let style_id = paragraph_style_id(paragraph);
    let style_name = style_id
        .as_ref()
        .and_then(|value| style_map.get(value).cloned())
        .or_else(|| style_id.clone());
    let paragraph_is_f8_cite = style_name.as_deref().map(is_f8_cite_style).unwrap_or(false);
    let runs = paragraph_runs(paragraph, style_map);
    let is_f8_cite = paragraph_is_f8_cite || runs.iter().any(|run| run.is_f8_cite);

    let mut level = detect_heading_level(paragraph, style_map);
    if level.is_some() && (is_probable_author_line(&text) || is_f8_cite) {
        level = None;
    }

    TexBlock {
        id: format!("p-{order}"),
        kind: if level.is_some() {
            "heading".to_string()
        } else {
            "paragraph".to_string()
        },
        text,
        runs,
        level,
        style_id,
        style_name,
        is_f8_cite,
    }
}

pub fn open_tex_document(file_path: &Path) -> CommandResult<TexDocumentPayload> {
    let file = File::open(file_path)
        .map_err(|error| format!("Could not open '{}': {error}", path_display(file_path)))?;
    let mut archive = ZipArchive::new(file)
        .map_err(|error| format!("Could not read '{}': {error}", path_display(file_path)))?;

    let document_xml = read_zip_file(&mut archive, "word/document.xml").ok_or_else(|| {
        format!(
            "Missing word/document.xml in '{}'. Is this a valid docx file?",
            path_display(file_path)
        )
    })?;
    let style_map = read_style_map(read_zip_file(&mut archive, "word/styles.xml"));

    let document = roxmltree::Document::parse(&document_xml).map_err(|error| {
        format!(
            "Could not parse XML in '{}': {error}",
            path_display(file_path)
        )
    })?;

    let mut blocks = Vec::new();
    for (index, paragraph) in document
        .descendants()
        .filter(|node| has_tag(*node, "p"))
        .enumerate()
    {
        blocks.push(build_tex_block(paragraph, index + 1, &style_map));
    }

    Ok(TexDocumentPayload {
        file_path: path_display(file_path),
        file_name: file_path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("Untitled.docx")
            .to_string(),
        paragraph_count: i64::try_from(blocks.len()).unwrap_or(0),
        blocks,
    })
}
