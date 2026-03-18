use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Cursor, Read, Write};
use std::path::Path;

use docx_rs::Docx;
use roxmltree::Node;
use zip::ZipArchive;

pub type DocxResult<T> = Result<T, String>;

const VERBATIM_TEMPLATE_DOTM: &[u8] = include_bytes!("../resources/Debate.dotm");
const VERBATIM_TEMPLATE_PARTS: &[&str] = &[
    "word/styles.xml",
    "word/theme/theme1.xml",
    "word/fontTable.xml",
    "word/settings.xml",
    "word/webSettings.xml",
    "word/numbering.xml",
];

pub fn normalize_for_search(text: &str) -> String {
    let mut normalized = String::with_capacity(text.len());
    let mut previous_space = false;
    for character in text.chars() {
        if character.is_alphanumeric() {
            previous_space = false;
            for lower in character.to_lowercase() {
                normalized.push(lower);
            }
        } else if !previous_space {
            normalized.push(' ');
            previous_space = true;
        }
    }
    normalized.trim().to_string()
}

pub fn path_display(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

pub fn has_tag(node: Node<'_, '_>, expected: &str) -> bool {
    node.is_element() && node.tag_name().name() == expected
}

pub fn attribute_value<'a>(node: Node<'a, 'a>, key: &str) -> Option<&'a str> {
    if let Some(value) = node.attribute(key) {
        return Some(value);
    }
    node.attributes()
        .find_map(|attribute| (attribute.name().ends_with(key)).then_some(attribute.value()))
}

pub fn parse_trailing_level(value: &str) -> Option<i64> {
    let lowered = value.to_ascii_lowercase();

    if let Some(without_h) = lowered.strip_prefix('h') {
        if let Ok(level) = without_h.parse::<i64>() {
            if (1..=9).contains(&level) {
                return Some(level);
            }
        }
    }

    if let Some(index) = lowered.find("heading") {
        let tail = &lowered[index + "heading".len()..];
        let digits: String = tail
            .chars()
            .filter(|character| character.is_ascii_digit())
            .collect();
        if let Ok(level) = digits.parse::<i64>() {
            if (1..=9).contains(&level) {
                return Some(level);
            }
        }
    }

    None
}

pub fn read_zip_file(archive: &mut ZipArchive<File>, entry_name: &str) -> Option<String> {
    let mut entry = archive.by_name(entry_name).ok()?;
    let mut value = String::new();
    entry.read_to_string(&mut value).ok()?;
    Some(value)
}

pub fn read_docx_part(path: &Path, part_name: &str) -> DocxResult<Option<String>> {
    let file =
        File::open(path).map_err(|error| format!("Could not open '{}': {error}", path_display(path)))?;
    let mut archive = ZipArchive::new(file)
        .map_err(|error| format!("Could not read '{}': {error}", path_display(path)))?;
    Ok(read_zip_file(&mut archive, part_name))
}

pub fn read_style_map(styles_xml: Option<String>) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let Some(styles_xml) = styles_xml else {
        return map;
    };

    let Ok(document) = roxmltree::Document::parse(&styles_xml) else {
        return map;
    };

    for style in document.descendants().filter(|node| has_tag(*node, "style")) {
        let Some(style_id) = attribute_value(style, "styleId") else {
            continue;
        };

        let mut display_name = style_id.to_string();
        if let Some(name_node) = style.children().find(|node| has_tag(*node, "name")) {
            if let Some(value) = attribute_value(name_node, "val") {
                display_name = value.to_string();
            }
        }

        map.insert(style_id.to_string(), display_name);
    }

    map
}

pub fn extract_paragraph_text(paragraph: Node<'_, '_>) -> String {
    let mut value = String::new();

    for node in paragraph.descendants().filter(|node| node.is_element()) {
        if has_tag(node, "t") {
            if let Some(text) = node.text() {
                value.push_str(text);
            }
        } else if has_tag(node, "tab") {
            value.push('\t');
        } else if has_tag(node, "br") || has_tag(node, "cr") {
            value.push('\n');
        }
    }

    value
}

pub fn run_has_property(run: Node<'_, '_>, property_tag: &str) -> bool {
    run.children()
        .find(|node| has_tag(*node, "rPr"))
        .and_then(|props| props.children().find(|node| has_tag(*node, property_tag)))
        .is_some()
}

pub fn run_style_id(run: Node<'_, '_>) -> Option<String> {
    let props = run.children().find(|node| has_tag(*node, "rPr"))?;
    let style = props.children().find(|node| has_tag(*node, "rStyle"))?;
    let style_id = attribute_value(style, "val")?;
    Some(style_id.to_string())
}

pub fn run_style_name(run: Node<'_, '_>, style_map: &HashMap<String, String>) -> Option<String> {
    let style_id = run_style_id(run)?;
    Some(style_map.get(&style_id).cloned().unwrap_or(style_id))
}

pub fn run_has_active_underline(run: Node<'_, '_>) -> bool {
    let Some(props) = run.children().find(|node| has_tag(*node, "rPr")) else {
        return false;
    };

    let Some(underline) = props.children().find(|node| has_tag(*node, "u")) else {
        return false;
    };

    let Some(value) = attribute_value(underline, "val") else {
        return true;
    };

    !(value.eq_ignore_ascii_case("none")
        || value.eq_ignore_ascii_case("false")
        || value.eq_ignore_ascii_case("0"))
}

pub fn run_highlight_class(run: Node<'_, '_>) -> Option<&'static str> {
    let props = run.children().find(|node| has_tag(*node, "rPr"))?;
    let highlight = props.children().find(|node| has_tag(*node, "highlight"))?;
    let value = attribute_value(highlight, "val")?
        .trim()
        .to_ascii_lowercase();

    match value.as_str() {
        "yellow" | "darkyellow" => Some("yellow"),
        "green" | "darkgreen" => Some("green"),
        "cyan" | "darkcyan" | "turquoise" => Some("cyan"),
        "magenta" | "darkmagenta" | "pink" => Some("magenta"),
        "blue" | "darkblue" => Some("blue"),
        "gray" | "grey" | "lightgray" | "darkgray" | "gray25" | "gray50" => Some("gray"),
        _ => None,
    }
}

pub fn detect_heading_level(
    paragraph: Node<'_, '_>,
    style_map: &HashMap<String, String>,
) -> Option<i64> {
    let paragraph_props = paragraph.children().find(|node| has_tag(*node, "pPr"))?;

    if let Some(outline_level_node) = paragraph_props
        .children()
        .find(|node| has_tag(*node, "outlineLvl"))
    {
        if let Some(raw_level) = attribute_value(outline_level_node, "val") {
            if let Ok(level_zero_based) = raw_level.parse::<i64>() {
                let level = level_zero_based + 1;
                if (1..=9).contains(&level) {
                    return Some(level);
                }
            }
        }
    }

    let style_node = paragraph_props
        .children()
        .find(|node| has_tag(*node, "pStyle"))?;
    let style_id = attribute_value(style_node, "val")?;

    if let Some(level) = parse_trailing_level(style_id) {
        return Some(level);
    }

    if let Some(style_name) = style_map.get(style_id) {
        return parse_trailing_level(style_name);
    }

    None
}

pub fn is_f8_cite_style(style_label: &str) -> bool {
    let normalized = normalize_for_search(style_label);
    normalized == "cite"
        || normalized.starts_with("cite ")
        || normalized.ends_with(" cite")
        || normalized.contains(" cite ")
        || normalized.contains("f8 cite")
        || normalized.contains("f8cite")
}

fn contains_year_token(normalized: &str) -> bool {
    for token in normalized.split_whitespace() {
        if let Ok(year) = token.parse::<i32>() {
            if (1900..=2099).contains(&year) {
                return true;
            }
        }
    }
    false
}

pub fn is_probable_author_line(text: &str) -> bool {
    let normalized = normalize_for_search(text);
    if normalized.is_empty() {
        return false;
    }

    let word_count = normalized.split_whitespace().count();
    if !(3..=90).contains(&word_count) {
        return false;
    }

    if !contains_year_token(&normalized) {
        return false;
    }

    let comma_count = text.matches(',').count();
    let has_source_marker = normalized.contains("journal")
        || normalized.contains("university")
        || normalized.contains("postdoctoral")
        || normalized.contains("vol ")
        || normalized.contains("edition")
        || normalized.contains("press")
        || normalized.contains("retrieved")
        || normalized.contains("archive");
    let looks_like_url_line = normalized.contains("http") || normalized.contains("doi");

    (comma_count >= 2 || has_source_marker || looks_like_url_line) && word_count >= 5
}

pub fn xml_escape_text(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

pub fn xml_escape_attr(value: &str) -> String {
    xml_escape_text(value)
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

pub fn paragraph_xml_plain(text: &str) -> String {
    if text.is_empty() {
        return "<w:p/>".to_string();
    }
    format!(
        "<w:p><w:r><w:t xml:space=\"preserve\">{}</w:t></w:r></w:p>",
        xml_escape_text(text)
    )
}

pub fn paragraph_xml_bold(text: &str) -> String {
    format!(
        "<w:p><w:r><w:rPr><w:b/></w:rPr><w:t xml:space=\"preserve\">{}</w:t></w:r></w:p>",
        xml_escape_text(text)
    )
}

pub fn paragraph_xml_heading(level: i64, text: &str) -> String {
    let style_id = format!("Heading{}", level);
    format!(
        "<w:p><w:pPr><w:pStyle w:val=\"{}\"/></w:pPr><w:r><w:t xml:space=\"preserve\">{}</w:t></w:r></w:p>",
        xml_escape_attr(&style_id),
        xml_escape_text(text)
    )
}

fn apply_verbatim_template_parts(capture_path: &Path) -> DocxResult<()> {
    let cursor = Cursor::new(VERBATIM_TEMPLATE_DOTM);
    let mut archive = ZipArchive::new(cursor)
        .map_err(|error| format!("Could not read bundled Verbatim template: {error}"))?;
    let mut replacements = HashMap::new();

    for part_name in VERBATIM_TEMPLATE_PARTS {
        let mut entry = archive.by_name(part_name).map_err(|error| {
            format!("Bundled Verbatim template is missing '{part_name}': {error}")
        })?;
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).map_err(|error| {
            format!("Could not read '{part_name}' from bundled Verbatim template: {error}")
        })?;
        replacements.insert((*part_name).to_string(), bytes);
    }

    rewrite_docx_with_parts(capture_path, &replacements)
}

pub fn create_blank_docx(capture_path: &Path) -> DocxResult<()> {
    let mut output = File::create(capture_path).map_err(|error| {
        format!(
            "Could not create capture docx '{}': {error}",
            path_display(capture_path)
        )
    })?;
    Docx::new().build().pack(&mut output).map_err(|error| {
        format!(
            "Could not initialize capture docx '{}': {error}",
            path_display(capture_path)
        )
    })?;

    apply_verbatim_template_parts(capture_path)
}

pub fn rewrite_docx_with_parts(
    capture_path: &Path,
    replacements: &HashMap<String, Vec<u8>>,
) -> DocxResult<()> {
    let source_file = File::open(capture_path).map_err(|error| {
        format!(
            "Could not open capture docx '{}' for update: {error}",
            path_display(capture_path)
        )
    })?;
    let mut archive = ZipArchive::new(source_file).map_err(|error| {
        format!(
            "Could not read capture docx '{}' for update: {error}",
            path_display(capture_path)
        )
    })?;

    let temp_path = capture_path.with_extension("docx.tmp");
    let temp_file = File::create(&temp_path).map_err(|error| {
        format!(
            "Could not create temporary capture file '{}': {error}",
            path_display(&temp_path)
        )
    })?;
    let mut writer = zip::ZipWriter::new(temp_file);
    let mut copied_names = std::collections::HashSet::new();

    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| format!("Could not read capture docx entry: {error}"))?;
        let name = entry.name().to_string();
        if entry.is_dir() {
            continue;
        }

        let options =
            zip::write::SimpleFileOptions::default().compression_method(entry.compression());
        writer
            .start_file(name.clone(), options)
            .map_err(|error| format!("Could not write capture zip entry '{name}': {error}"))?;

        if let Some(updated_bytes) = replacements.get(&name) {
            writer
                .write_all(updated_bytes)
                .map_err(|error| format!("Could not write capture zip entry '{name}': {error}"))?;
        } else {
            let mut original = Vec::new();
            entry
                .read_to_end(&mut original)
                .map_err(|error| format!("Could not read capture zip entry '{name}': {error}"))?;
            writer
                .write_all(&original)
                .map_err(|error| format!("Could not write capture zip entry '{name}': {error}"))?;
        }

        copied_names.insert(name);
    }

    for (name, updated_bytes) in replacements {
        if copied_names.contains(name) {
            continue;
        }

        writer
            .start_file(name, zip::write::SimpleFileOptions::default())
            .map_err(|error| format!("Could not add capture zip entry '{name}': {error}"))?;
        writer
            .write_all(updated_bytes)
            .map_err(|error| format!("Could not add capture zip entry '{name}': {error}"))?;
    }

    writer
        .finish()
        .map_err(|error| format!("Could not finish capture zip rewrite: {error}"))?;

    match fs::rename(&temp_path, capture_path) {
        Ok(()) => Ok(()),
        Err(_) => {
            fs::remove_file(capture_path).map_err(|error| {
                format!(
                    "Could not replace capture docx '{}': {error}",
                    path_display(capture_path)
                )
            })?;
            fs::rename(&temp_path, capture_path).map_err(|error| {
                format!(
                    "Could not move updated capture docx into place '{}': {error}",
                    path_display(capture_path)
                )
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};
    use zip::write::SimpleFileOptions;

    fn temp_path(label: &str) -> std::path::PathBuf {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("docx-{label}-{timestamp}.docx"))
    }

    #[test]
    fn normalizes_citation_styles() {
        assert!(is_f8_cite_style("F8 Cite"));
        assert!(is_f8_cite_style("Cite"));
        assert!(!is_f8_cite_style("Heading 2"));
    }

    #[test]
    fn detects_heading_level_from_style_map() {
        let xml = r#"
            <w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
              <w:body>
                <w:p>
                  <w:pPr><w:pStyle w:val="Head2"/></w:pPr>
                  <w:r><w:t>Heading</w:t></w:r>
                </w:p>
              </w:body>
            </w:document>
        "#;
        let document = roxmltree::Document::parse(xml).unwrap();
        let paragraph = document.descendants().find(|node| has_tag(*node, "p")).unwrap();
        let style_map = HashMap::from([(String::from("Head2"), String::from("Heading 2"))]);
        assert_eq!(detect_heading_level(paragraph, &style_map), Some(2));
    }

    #[test]
    fn can_create_and_rewrite_blank_docx() {
        let path = temp_path("rewrite");
        create_blank_docx(&path).unwrap();
        let replacement = String::from(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>Hello</w:t></w:r></w:p></w:body></w:document>"#,
        );
        let replacements = HashMap::from([("word/document.xml".to_string(), replacement.into_bytes())]);
        rewrite_docx_with_parts(&path, &replacements).unwrap();
        let updated = read_docx_part(&path, "word/document.xml").unwrap().unwrap();
        assert!(updated.contains("Hello"));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn missing_parts_return_none_without_failing() {
        let path = temp_path("missing-part");
        create_blank_docx(&path).unwrap();

        let missing = read_docx_part(&path, "word/does-not-exist.xml").unwrap();
        assert_eq!(missing, None);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn invalid_zip_reports_stable_error() {
        let path = temp_path("invalid");
        fs::write(&path, b"not a zip archive").unwrap();

        let error = read_docx_part(&path, "word/document.xml").unwrap_err();
        assert!(error.contains("Could not read"));

        let _ = fs::remove_file(path);
    }

    #[test]
    fn highlight_aliases_map_to_expected_classes() {
        let xml = r#"
            <w:r xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
              <w:rPr><w:highlight w:val="darkYellow"/></w:rPr>
              <w:t>text</w:t>
            </w:r>
        "#;
        let document = roxmltree::Document::parse(xml).unwrap();
        assert_eq!(run_highlight_class(document.root_element()), Some("yellow"));
    }

    #[test]
    fn detects_heading_level_from_outline_level() {
        let xml = r#"
            <w:p xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
              <w:pPr><w:outlineLvl w:val="2"/></w:pPr>
              <w:r><w:t>Heading</w:t></w:r>
            </w:p>
        "#;
        let document = roxmltree::Document::parse(xml).unwrap();
        assert_eq!(detect_heading_level(document.root_element(), &HashMap::new()), Some(3));
    }

    #[test]
    fn rewrite_docx_with_parts_updates_minimal_archive() {
        let path = temp_path("minimal");
        {
            let file = File::create(&path).unwrap();
            let mut writer = zip::ZipWriter::new(file);
            writer
                .start_file("word/document.xml", SimpleFileOptions::default())
                .unwrap();
            writer
                .write_all(
                    br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p/></w:body></w:document>"#,
                )
                .unwrap();
            writer.finish().unwrap();
        }

        let replacements = HashMap::from([(
            "word/document.xml".to_string(),
            br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>Minimal</w:t></w:r></w:p></w:body></w:document>"#.to_vec(),
        )]);
        rewrite_docx_with_parts(&path, &replacements).unwrap();

        let updated = read_docx_part(&path, "word/document.xml").unwrap().unwrap();
        assert!(updated.contains("Minimal"));

        let _ = fs::remove_file(path);
    }
}
