//! Whisper model files (ggml format from the whisper.cpp project). Downloaded
//! once on first use into %LOCALAPPDATA%\AgentNotch\models.

use std::io::{Read, Write};
use std::path::PathBuf;

use crate::paths;

#[derive(Debug, thiserror::Error)]
pub enum ModelError {
    #[error("model.download")]
    Download,
    #[error("model.invalid")]
    Invalid,
    #[error("model.io")]
    Io(#[from] std::io::Error),
}

pub fn file_name(model: &str) -> String {
    format!("ggml-{model}.bin")
}

pub fn path(model: &str) -> PathBuf {
    paths::models_dir().join(file_name(model))
}

pub fn url(model: &str) -> String {
    format!(
        "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{}",
        file_name(model)
    )
}

/// whisper.cpp ggml files start with the magic 0x67676d6c ("ggml") stored
/// little-endian.
pub fn has_ggml_magic(header: &[u8]) -> bool {
    header.len() >= 4 && header[..4] == [0x6c, 0x6d, 0x67, 0x67]
}

pub fn is_installed(model: &str) -> bool {
    let Ok(mut file) = std::fs::File::open(path(model)) else {
        return false;
    };
    let mut header = [0u8; 4];
    file.read_exact(&mut header).is_ok() && has_ggml_magic(&header)
}

/// Downloads to a `.part` file, validates it and renames it into place.
/// `progress` receives 0...100 when the server sends a length.
pub fn download(model: &str, mut progress: impl FnMut(u8)) -> Result<PathBuf, ModelError> {
    let target = path(model);
    std::fs::create_dir_all(paths::models_dir())?;
    let part = target.with_extension("bin.part");

    let mut response = ureq::get(&url(model))
        .call()
        .map_err(|_| ModelError::Download)?;
    let total: Option<u64> = response
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok());
    let mut reader = response
        .body_mut()
        .with_config()
        .limit(4 * 1024 * 1024 * 1024)
        .reader();
    let mut file = std::fs::File::create(&part)?;
    let mut buf = vec![0u8; 256 * 1024];
    let mut written: u64 = 0;
    let mut last_pct = u8::MAX;
    loop {
        let n = reader.read(&mut buf).map_err(|_| ModelError::Download)?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
        written += n as u64;
        if let Some(total) = total.filter(|t| *t > 0) {
            let pct = ((written * 100) / total).min(100) as u8;
            if pct != last_pct {
                last_pct = pct;
                progress(pct);
            }
        }
    }
    file.sync_all()?;
    drop(file);

    let mut header = [0u8; 4];
    let valid = std::fs::File::open(&part)
        .and_then(|mut f| f.read_exact(&mut header))
        .is_ok()
        && has_ggml_magic(&header)
        && total.is_none_or(|t| t == written);
    if !valid {
        let _ = std::fs::remove_file(&part);
        return Err(ModelError::Invalid);
    }
    std::fs::rename(&part, &target)?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn magic_detection() {
        assert!(has_ggml_magic(b"lmgg\x00\x01"));
        assert!(!has_ggml_magic(b"<!DOCTYPE html>"));
        assert!(!has_ggml_magic(b"lm"));
    }

    #[test]
    fn model_urls() {
        assert_eq!(
            url("base"),
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin"
        );
        assert!(
            path("small").ends_with("models/ggml-small.bin")
                || path("small").ends_with(r"models\ggml-small.bin")
        );
    }
}
