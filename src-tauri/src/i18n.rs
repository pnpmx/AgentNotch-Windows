//! The UI and the tray menu share `ui/i18n.json`, so there is one table.

use std::collections::HashMap;
use std::sync::OnceLock;

type Table = HashMap<String, HashMap<String, String>>;

fn table() -> &'static Table {
    static TABLE: OnceLock<Table> = OnceLock::new();
    TABLE.get_or_init(|| {
        serde_json::from_str(include_str!("../../ui/i18n.json")).expect("valid i18n.json")
    })
}

pub fn t(language: &str, key: &str) -> String {
    let table = table();
    table
        .get(language)
        .and_then(|l| l.get(key))
        .or_else(|| table.get("en").and_then(|l| l.get(key)))
        .cloned()
        .unwrap_or_else(|| key.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::LANGUAGES;

    fn placeholders(text: &str) -> Vec<String> {
        let mut found: Vec<String> = text
            .split('{')
            .skip(1)
            .filter_map(|part| part.split_once('}').map(|(name, _)| name.to_owned()))
            .collect();
        found.sort();
        found
    }

    #[test]
    fn every_language_has_every_key_with_same_placeholders() {
        let table = table();
        let english = &table["en"];
        for language in LANGUAGES {
            let entries = table
                .get(language)
                .unwrap_or_else(|| panic!("missing language {language}"));
            for (key, text) in english {
                let translated = entries
                    .get(key)
                    .unwrap_or_else(|| panic!("{language} missing {key}"));
                assert!(!translated.is_empty(), "{language} empty {key}");
                assert_eq!(
                    placeholders(translated),
                    placeholders(text),
                    "{language} placeholders for {key}"
                );
            }
            assert_eq!(entries.len(), english.len(), "{language} has extra keys");
        }
    }

    #[test]
    fn falls_back_to_english_then_key() {
        assert_eq!(t("xx", "copy"), "Copy");
        assert_eq!(t("es", "nope"), "nope");
    }

    #[test]
    fn backend_error_keys_are_translated() {
        let keys = [
            "audio.noDevice",
            "audio.format",
            "audio.stream",
            "model.download",
            "model.invalid",
            "model.io",
            "model.load",
            "transcribe.failed",
            "codex.notFound",
            "codex.launch",
            "codex.timeout",
            "codex.closed",
            "codex.rejected",
            "codex.noLimits",
            "claude.bridge.executable",
            "claude.bridge.existing",
            "claude.bridge.malformed",
        ];
        for key in keys {
            assert!(table()["en"].contains_key(key), "missing {key}");
        }
    }
}
