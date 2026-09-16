use toml_edit::{Array, DocumentMut, Item, Table, Value};

pub(crate) const STATUS_LINE_ITEMS: [&str; 6] = [
    "current-dir",
    "model-with-reasoning",
    "context-used",
    "used-tokens",
    "total-input-tokens",
    "total-output-tokens",
];

pub(crate) const STATUS_LINE_TOML: &str = r#"[tui]
status_line = [
  "current-dir",
  "model-with-reasoning",
  "context-used",
  "used-tokens",
  "total-input-tokens",
  "total-output-tokens"
]
"#;

pub(crate) fn apply_status_line(document: &mut DocumentMut) -> Result<(), ()> {
    if !document.contains_key("tui") {
        document.insert("tui", Item::Table(Table::new()));
    }
    let tui = document
        .get_mut("tui")
        .and_then(Item::as_table_like_mut)
        .ok_or(())?;
    let mut status_line = Array::new();
    for item in STATUS_LINE_ITEMS {
        status_line.push(item);
    }
    tui.insert("status_line", Item::Value(Value::Array(status_line)));
    Ok(())
}

pub(crate) fn has_expected_status_line(document: &DocumentMut) -> bool {
    let Some(array) = document
        .get("tui")
        .and_then(Item::as_table_like)
        .and_then(|tui| tui.get("status_line"))
        .and_then(Item::as_array)
    else {
        return false;
    };
    array
        .iter()
        .map(|value| value.as_str())
        .eq(STATUS_LINE_ITEMS.iter().copied().map(Some))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_line_update_preserves_other_tui_settings() {
        let mut document = r#"custom = true

[tui]
notifications = false
status_line = ["current-dir"]
"#
        .parse::<DocumentMut>()
        .expect("fixture TOML");

        apply_status_line(&mut document).expect("apply status line");

        assert_eq!(document["custom"].as_bool(), Some(true));
        assert_eq!(document["tui"]["notifications"].as_bool(), Some(false));
        assert_eq!(
            document["tui"]["status_line"]
                .as_array()
                .expect("status line array")
                .iter()
                .filter_map(|value| value.as_str())
                .collect::<Vec<_>>(),
            STATUS_LINE_ITEMS
        );
    }

    #[test]
    fn non_table_tui_is_rejected() {
        let mut document = "tui = false\n"
            .parse::<DocumentMut>()
            .expect("fixture TOML");

        assert_eq!(apply_status_line(&mut document), Err(()));
        assert_eq!(document["tui"].as_bool(), Some(false));
    }
}
