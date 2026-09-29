use serde::{Deserialize, Serialize};

use crate::dock::Edge;
use crate::paths;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// "system" or a language code (en, es, it, fr, de, pt).
    pub ui_language: String,
    /// "auto" or a Whisper language code.
    pub dictation_language: String,
    /// Whisper model name: "base" or "small".
    pub model: String,
    /// Hold-to-talk shortcut in tauri-plugin-global-shortcut syntax.
    pub hotkey: String,
    pub edge: Edge,
    /// Position along the edge, 0...1.
    pub offset: f64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            ui_language: "system".into(),
            dictation_language: "auto".into(),
            model: "base".into(),
            hotkey: "Ctrl+Shift+Space".into(),
            edge: Edge::Right,
            offset: 0.3,
        }
    }
}

pub const LANGUAGES: [&str; 6] = ["en", "es", "it", "fr", "de", "pt"];
pub const MODELS: [&str; 2] = ["base", "small"];

impl Settings {
    pub fn load() -> Self {
        std::fs::read(paths::settings())
            .ok()
            .and_then(|d| serde_json::from_slice::<Settings>(&d).ok())
            .map(Settings::sanitized)
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let path = paths::settings();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(data) = serde_json::to_vec_pretty(self) {
            let tmp = path.with_extension("json.tmp");
            if std::fs::write(&tmp, data).is_ok() {
                let _ = std::fs::rename(tmp, path);
            }
        }
    }

    fn sanitized(mut self) -> Self {
        let defaults = Settings::default();
        if self.ui_language != "system" && !LANGUAGES.contains(&self.ui_language.as_str()) {
            self.ui_language = defaults.ui_language;
        }
        if self.dictation_language != "auto"
            && !LANGUAGES.contains(&self.dictation_language.as_str())
        {
            self.dictation_language = defaults.dictation_language;
        }
        if !MODELS.contains(&self.model.as_str()) {
            self.model = defaults.model;
        }
        if !self.offset.is_finite() {
            self.offset = defaults.offset;
        }
        self.offset = self.offset.clamp(0.0, 1.0);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_values_fall_back() {
        let raw = r#"{"uiLanguage":"xx","dictationLanguage":"klingon","model":"huge","offset":7,"edge":"top"}"#;
        let s = serde_json::from_str::<Settings>(raw).unwrap().sanitized();
        assert_eq!(s.ui_language, "system");
        assert_eq!(s.dictation_language, "auto");
        assert_eq!(s.model, "base");
        assert_eq!(s.offset, 1.0);
        assert_eq!(s.edge, Edge::Top);
    }

    #[test]
    fn missing_fields_use_defaults() {
        let s = serde_json::from_str::<Settings>("{}").unwrap();
        assert_eq!(s, Settings::default());
    }
}
