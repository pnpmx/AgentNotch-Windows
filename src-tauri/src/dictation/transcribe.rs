use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use super::model;

#[derive(Debug, thiserror::Error)]
pub enum TranscribeError {
    #[error("model.load")]
    Load,
    #[error("transcribe.failed")]
    Failed,
}

/// Keeps one loaded model so consecutive dictations start instantly.
#[derive(Default)]
pub struct Transcriber {
    loaded: Option<(String, WhisperContext)>,
}

impl Transcriber {
    fn context(&mut self, model_name: &str) -> Result<&WhisperContext, TranscribeError> {
        if self
            .loaded
            .as_ref()
            .is_none_or(|(name, _)| name != model_name)
        {
            self.loaded = None;
            let ctx = WhisperContext::new_with_params(
                model::path(model_name),
                WhisperContextParameters::default(),
            )
            .map_err(|_| TranscribeError::Load)?;
            self.loaded = Some((model_name.to_owned(), ctx));
        }
        Ok(&self.loaded.as_ref().expect("loaded above").1)
    }

    /// `language` is a Whisper code or "auto".
    pub fn transcribe(
        &mut self,
        model_name: &str,
        language: &str,
        vocabulary: &str,
        samples: &[f32],
    ) -> Result<String, TranscribeError> {
        let ctx = self.context(model_name)?;
        let mut state = ctx.create_state().map_err(|_| TranscribeError::Load)?;
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_language(Some(language));
        params.set_translate(false);
        params.set_no_context(true);
        params.set_suppress_blank(true);
        // Whisper treats the initial prompt as preceding text, which biases it
        // towards these spellings (project names, jargon).
        let vocabulary = vocabulary.replace('\0', "");
        if !vocabulary.trim().is_empty() {
            params.set_initial_prompt(&vocabulary);
        }
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        let threads = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
            .clamp(1, 8);
        params.set_n_threads(threads as i32);
        state
            .full(params, samples)
            .map_err(|_| TranscribeError::Failed)?;
        let text: Vec<String> = state
            .as_iter()
            .filter_map(|segment| segment.to_str_lossy().ok().map(|s| s.trim().to_owned()))
            .filter(|s| !s.is_empty() && !is_non_speech_marker(s))
            .collect();
        Ok(text.join(" "))
    }
}

/// Whisper emits bracketed annotations such as "[BLANK_AUDIO]" or "(music)"
/// for silence and noise; they must never be pasted.
pub fn is_non_speech_marker(segment: &str) -> bool {
    let s = segment.trim();
    (s.starts_with('[') && s.ends_with(']')) || (s.starts_with('(') && s.ends_with(')'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markers_are_filtered() {
        assert!(is_non_speech_marker("[BLANK_AUDIO]"));
        assert!(is_non_speech_marker(" (música) "));
        assert!(!is_non_speech_marker("Hola, ¿qué tal?"));
    }
}
