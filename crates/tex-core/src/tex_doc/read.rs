use std::collections::HashMap;
use std::fs::File;
use std::path::Path;

use docx::{
    attribute_value, detect_heading_level, extract_paragraph_text, has_tag, is_f8_cite_style,
    is_probable_author_line, path_display, read_style_map, read_zip_file, run_has_active_underline,
    run_has_property, run_highlight_class, run_style_id, run_style_name,
};
use roxmltree::Node;
use zip::ZipArchive;

use crate::{CommandResult, TexBlock, TexDocumentPayload, TexTextRun};

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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::fs::File;
    use std::io::Write;

    use docx::{create_blank_docx, rewrite_docx_with_parts};
    use zip::write::SimpleFileOptions;

    use super::open_tex_document;

    fn temp_path(label: &str) -> std::path::PathBuf {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("tex-read-{label}-{timestamp}.docx"))
    }

    #[test]
    fn opens_headings_and_formats_runs() {
        let path = temp_path("formats");
        create_blank_docx(&path).unwrap();
        let document_xml = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p>
      <w:pPr><w:pStyle w:val="Heading2"/></w:pPr>
      <w:r><w:t>Case</w:t></w:r>
    </w:p>
    <w:p>
      <w:r>
        <w:rPr><w:b/><w:highlight w:val="yellow"/></w:rPr>
        <w:t>Tagged</w:t>
      </w:r>
    </w:p>
    <w:sectPr/>
  </w:body>
</w:document>"#;
        let styles_xml = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:style w:type="paragraph" w:styleId="Heading2"><w:name w:val="Heading 2"/></w:style>
</w:styles>"#;
        let replacements = HashMap::from([
            ("word/document.xml".to_string(), document_xml.as_bytes().to_vec()),
            ("word/styles.xml".to_string(), styles_xml.as_bytes().to_vec()),
        ]);
        rewrite_docx_with_parts(&path, &replacements).unwrap();

        let document = open_tex_document(&path).unwrap();
        assert_eq!(document.paragraph_count, 2);
        assert_eq!(document.blocks[0].kind, "heading");
        assert_eq!(document.blocks[0].level, Some(2));
        assert!(document.blocks[1].runs[0].bold);
        assert_eq!(document.blocks[1].runs[0].highlight_color.as_deref(), Some("yellow"));

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn author_lines_do_not_become_headings() {
        let path = temp_path("author-line");
        create_blank_docx(&path).unwrap();
        let document_xml = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p>
      <w:pPr><w:pStyle w:val="Heading2"/></w:pPr>
      <w:r><w:t>Jane Doe, Example University, 2024 archive edition</w:t></w:r>
    </w:p>
    <w:sectPr/>
  </w:body>
</w:document>"#;
        let replacements =
            HashMap::from([("word/document.xml".to_string(), document_xml.as_bytes().to_vec())]);
        rewrite_docx_with_parts(&path, &replacements).unwrap();

        let document = open_tex_document(&path).unwrap();
        assert_eq!(document.blocks[0].kind, "paragraph");
        assert_eq!(document.blocks[0].level, None);

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn rejects_invalid_xml_with_stable_error() {
        let path = temp_path("invalid-xml");
        create_blank_docx(&path).unwrap();
        let replacements = HashMap::from([("word/document.xml".to_string(), b"<w:document".to_vec())]);
        rewrite_docx_with_parts(&path, &replacements).unwrap();

        let error = open_tex_document(&path).unwrap_err();
        assert!(error.contains("Could not parse XML"));

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn rejects_missing_document_xml() {
        let path = temp_path("missing-document");
        let file = File::create(&path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        writer
            .start_file("[Content_Types].xml", SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"<Types/>").unwrap();
        writer.finish().unwrap();

        let error = open_tex_document(&path).unwrap_err();
        assert!(error.contains("Missing word/document.xml"));

        let _ = std::fs::remove_file(path);
    }
}
