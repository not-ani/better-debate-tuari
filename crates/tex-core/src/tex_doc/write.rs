use std::collections::HashMap;
use std::fs::File;
use std::path::Path;

use docx::{
    create_blank_docx, has_tag, path_display, read_zip_file, rewrite_docx_with_parts,
    xml_escape_attr, xml_escape_text,
};

use crate::{CommandResult, TexBlock, TexDocumentPayload, TexTextRun};

fn canonical_paragraph_style_id(block: &TexBlock) -> String {
    if block.kind == "heading" {
        let level = block.level.unwrap_or(1).clamp(1, 9);
        return format!("Heading{level}");
    }

    block.style_id.clone().unwrap_or_default()
}

fn text_nodes_xml(text: &str) -> String {
    let mut output = String::new();
    let mut buffer = String::new();

    let flush_text = |buffer: &mut String, output: &mut String| {
        if buffer.is_empty() {
            return;
        }
        output.push_str("<w:t xml:space=\"preserve\">");
        output.push_str(&xml_escape_text(buffer));
        output.push_str("</w:t>");
        buffer.clear();
    };

    for character in text.chars() {
        match character {
            '\n' => {
                flush_text(&mut buffer, &mut output);
                output.push_str("<w:br/>");
            }
            '\t' => {
                flush_text(&mut buffer, &mut output);
                output.push_str("<w:tab/>");
            }
            _ => buffer.push(character),
        }
    }

    flush_text(&mut buffer, &mut output);
    output
}

fn run_xml(run: &TexTextRun) -> String {
    let mut props = String::new();
    let run_style_id = run
        .style_id
        .clone()
        .filter(|value| !value.is_empty())
        .or_else(|| run.is_f8_cite.then(|| "Cite".to_string()));
    if let Some(style_id) = run_style_id {
        props.push_str("<w:rStyle w:val=\"");
        props.push_str(&xml_escape_attr(&style_id));
        props.push_str("\"/>");
    }
    if run.bold {
        props.push_str("<w:b/>");
    }
    if run.italic {
        props.push_str("<w:i/>");
    }
    if run.underline {
        props.push_str("<w:u w:val=\"single\"/>");
    }
    if run.small_caps {
        props.push_str("<w:smallCaps/>");
    }
    if let Some(color) = run
        .highlight_color
        .as_ref()
        .filter(|value| !value.is_empty())
    {
        props.push_str("<w:highlight w:val=\"");
        props.push_str(&xml_escape_attr(color));
        props.push_str("\"/>");
    }

    let body = text_nodes_xml(&run.text);
    if body.is_empty() {
        return String::new();
    }

    if props.is_empty() {
        return format!("<w:r>{body}</w:r>");
    }

    format!("<w:r><w:rPr>{props}</w:rPr>{body}</w:r>")
}

fn block_xml(block: &TexBlock) -> String {
    let level = (block.kind == "heading")
        .then_some(block.level.unwrap_or(1))
        .filter(|value| (1..=9).contains(value));
    let style_id = canonical_paragraph_style_id(block);

    let mut paragraph_props = String::new();
    if !style_id.is_empty() {
        paragraph_props.push_str("<w:pStyle w:val=\"");
        paragraph_props.push_str(&xml_escape_attr(&style_id));
        paragraph_props.push_str("\"/>");
    }
    if let Some(level) = level {
        paragraph_props.push_str("<w:outlineLvl w:val=\"");
        paragraph_props.push_str(&(level - 1).to_string());
        paragraph_props.push_str("\"/>");
    }

    let mut run_xml_segments = block.runs.iter().map(run_xml).collect::<Vec<_>>();
    if run_xml_segments.iter().all(|segment| segment.is_empty()) {
        if block.text.is_empty() {
            run_xml_segments = vec!["<w:r/>".to_string()];
        } else {
            run_xml_segments = vec![run_xml(&TexTextRun {
                text: block.text.clone(),
                bold: false,
                italic: false,
                underline: false,
                small_caps: false,
                highlight_color: None,
                style_id: None,
                style_name: None,
                is_f8_cite: false,
            })];
        }
    }

    if paragraph_props.is_empty() {
        return format!("<w:p>{}</w:p>", run_xml_segments.join(""));
    }

    format!(
        "<w:p><w:pPr>{}</w:pPr>{}</w:p>",
        paragraph_props,
        run_xml_segments.join("")
    )
}

fn document_with_updated_body(document_xml: &str, blocks: &[TexBlock]) -> CommandResult<String> {
    let document = roxmltree::Document::parse(document_xml)
        .map_err(|error| format!("Could not parse DOCX document XML: {error}"))?;
    let body = document
        .descendants()
        .find(|node| has_tag(*node, "body"))
        .ok_or_else(|| "Missing body node in DOCX document.".to_string())?;

    let body_range = body.range();
    let body_slice = &document_xml[body_range.clone()];
    let open_offset = body_slice
        .find('>')
        .ok_or_else(|| "Could not resolve DOCX body open tag.".to_string())?;
    let close_offset = body_slice
        .rfind("</")
        .ok_or_else(|| "Could not resolve DOCX body close tag.".to_string())?;

    let body_open_end = body_range.start + open_offset + 1;
    let body_close_start = body_range.start + close_offset;

    let section_props = body
        .children()
        .find(|node| has_tag(*node, "sectPr"))
        .map(|node| document_xml[node.range()].to_string())
        .unwrap_or_default();

    let mut updated = String::new();
    updated.push_str(&document_xml[..body_open_end]);
    updated.push('\n');
    for block in blocks {
        updated.push_str(&block_xml(block));
    }
    if !section_props.is_empty() {
        updated.push_str(&section_props);
    }
    updated.push_str(&document_xml[body_close_start..]);
    Ok(updated)
}

pub fn save_tex_document(
    file_path: &Path,
    blocks: &[TexBlock],
) -> CommandResult<TexDocumentPayload> {
    if !file_path.exists() {
        create_blank_docx(file_path)?;
    }

    let file = File::open(file_path)
        .map_err(|error| format!("Could not open '{}': {error}", path_display(file_path)))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|error| format!("Could not read '{}': {error}", path_display(file_path)))?;
    let document_xml = read_zip_file(&mut archive, "word/document.xml").ok_or_else(|| {
        format!(
            "Missing word/document.xml in '{}'. Is this a valid docx file?",
            path_display(file_path)
        )
    })?;

    let updated_document_xml = document_with_updated_body(&document_xml, blocks)?;
    let mut replacements = HashMap::new();
    replacements.insert(
        "word/document.xml".to_string(),
        updated_document_xml.into_bytes(),
    );
    rewrite_docx_with_parts(file_path, &replacements)?;

    super::open_tex_document(file_path)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{canonical_paragraph_style_id, save_tex_document};
    use crate::{TexBlock, TexDocumentPayload, TexTextRun};

    fn sample_block(kind: &str, level: Option<i64>, style_id: Option<&str>) -> TexBlock {
        TexBlock {
            id: "block-1".to_string(),
            kind: kind.to_string(),
            text: String::new(),
            runs: Vec::new(),
            level,
            style_id: style_id.map(str::to_string),
            style_name: None,
            is_f8_cite: false,
        }
    }

    #[test]
    fn heading_blocks_always_use_builtin_word_heading_styles() {
        assert_eq!(
            canonical_paragraph_style_id(&sample_block("heading", Some(2), Some("Hat"))),
            "Heading2"
        );
        assert_eq!(
            canonical_paragraph_style_id(&sample_block("heading", Some(3), Some("Block"))),
            "Heading3"
        );
    }

    #[test]
    fn paragraph_blocks_keep_existing_style_ids() {
        assert_eq!(
            canonical_paragraph_style_id(&sample_block("paragraph", None, Some("Cite"))),
            "Cite"
        );
    }

    #[test]
    fn save_round_trips_document_payload() {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("tex-write-{timestamp}.docx"));
        let blocks = vec![
            TexBlock {
                id: "p-1".to_string(),
                kind: "heading".to_string(),
                text: "Overview".to_string(),
                runs: vec![TexTextRun {
                    text: "Overview".to_string(),
                    bold: false,
                    italic: false,
                    underline: false,
                    small_caps: false,
                    highlight_color: None,
                    style_id: None,
                    style_name: None,
                    is_f8_cite: false,
                }],
                level: Some(2),
                style_id: Some("Heading2".to_string()),
                style_name: Some("Heading 2".to_string()),
                is_f8_cite: false,
            },
            TexBlock {
                id: "p-2".to_string(),
                kind: "paragraph".to_string(),
                text: "Body text".to_string(),
                runs: vec![TexTextRun {
                    text: "Body text".to_string(),
                    bold: true,
                    italic: false,
                    underline: false,
                    small_caps: false,
                    highlight_color: Some("yellow".to_string()),
                    style_id: None,
                    style_name: None,
                    is_f8_cite: false,
                }],
                level: None,
                style_id: None,
                style_name: None,
                is_f8_cite: false,
            },
        ];

        let saved: TexDocumentPayload = save_tex_document(Path::new(&path), &blocks).unwrap();
        assert_eq!(saved.paragraph_count, 2);
        assert_eq!(saved.blocks[0].kind, "heading");
        assert_eq!(saved.blocks[0].level, Some(2));
        assert_eq!(saved.blocks[1].runs[0].highlight_color.as_deref(), Some("yellow"));

        let _ = std::fs::remove_file(path);
    }
}
