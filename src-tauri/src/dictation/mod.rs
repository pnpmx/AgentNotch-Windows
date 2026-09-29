//! Push-to-talk worker. Owns the microphone stream and the Whisper model on a
//! dedicated thread; the UI thread only sends commands and receives events.

pub mod audio;
pub mod inject;
pub mod model;
pub mod transcribe;

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Serialize;

use audio::Recorder;
use inject::{PasteResult, Target};
use transcribe::Transcriber;

/// Presses shorter than this are treated as accidental taps.
pub const MIN_HOLD: Duration = Duration::from_millis(300);
const MIN_AUDIO_SECONDS: f64 = 0.3;

pub enum Command {
    Start {
        target: Option<Target>,
        model: String,
    },
    Stop {
        language: String,
        model: String,
    },
    /// Sent by the download thread when it ends.
    DownloadFinished {
        ok: bool,
        error: Option<String>,
    },
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum SpeechState {
    Idle,
    Listening,
    Transcribing,
    Downloading { percent: u8 },
    Failed { key: String },
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Event {
    State(SpeechState),
    /// `key` is an i18n key rendered by the UI.
    Notice {
        key: String,
    },
    Transcript {
        text: String,
    },
}

type Emit = Arc<dyn Fn(Event) + Send + Sync>;

pub fn spawn(emit: impl Fn(Event) + Send + Sync + 'static) -> Sender<Command> {
    let (tx, rx) = mpsc::channel();
    let worker_tx = tx.clone();
    let emit: Emit = Arc::new(emit);
    std::thread::Builder::new()
        .name("dictation".into())
        .spawn(move || Worker::default().run(rx, worker_tx, emit))
        .expect("spawn dictation thread");
    tx
}

fn start_download(name: String, tx: Sender<Command>, emit: Emit) {
    std::thread::spawn(move || {
        emit(Event::State(SpeechState::Downloading { percent: 0 }));
        let result = model::download(&name, |percent| {
            emit(Event::State(SpeechState::Downloading { percent }))
        });
        let _ = tx.send(Command::DownloadFinished {
            ok: result.is_ok(),
            error: result.err().map(|e| e.to_string()),
        });
    });
}

#[derive(Default)]
struct Worker {
    recorder: Option<(Recorder, Instant, Option<Target>)>,
    transcriber: Transcriber,
    downloading: bool,
}

impl Worker {
    fn run(mut self, rx: Receiver<Command>, tx: Sender<Command>, emit: Emit) {
        let notice = |key: &str| emit(Event::Notice { key: key.into() });
        while let Ok(command) = rx.recv() {
            match command {
                Command::Start {
                    target,
                    model: name,
                } => {
                    if self.recorder.is_some() {
                        continue;
                    }
                    if !model::is_installed(&name) {
                        if !self.downloading {
                            self.downloading = true;
                            start_download(name, tx.clone(), emit.clone());
                        }
                        notice("notice.modelDownloading");
                        continue;
                    }
                    match Recorder::start() {
                        Ok(recorder) => {
                            self.recorder = Some((recorder, Instant::now(), target));
                            emit(Event::State(SpeechState::Listening));
                        }
                        Err(error) => emit(Event::State(SpeechState::Failed {
                            key: error.to_string(),
                        })),
                    }
                }
                Command::Stop {
                    language,
                    model: name,
                } => {
                    let Some((recorder, started, target)) = self.recorder.take() else {
                        continue;
                    };
                    let samples = recorder.finish();
                    if started.elapsed() < MIN_HOLD
                        || audio::duration_seconds(&samples) < MIN_AUDIO_SECONDS
                    {
                        emit(Event::State(SpeechState::Idle));
                        notice("notice.tooShort");
                        continue;
                    }
                    emit(Event::State(SpeechState::Transcribing));
                    match self.transcriber.transcribe(&name, &language, &samples) {
                        Ok(text) if text.is_empty() => {
                            emit(Event::State(SpeechState::Idle));
                            notice("notice.noText");
                        }
                        Ok(text) => {
                            let result = inject::paste(&text, target);
                            if result != PasteResult::Attempted {
                                inject::copy_to_clipboard(&text);
                            }
                            emit(Event::Transcript { text });
                            emit(Event::State(SpeechState::Idle));
                            notice(match result {
                                PasteResult::Attempted => "notice.pasted",
                                PasteResult::DestinationChanged => "notice.destinationChanged",
                                PasteResult::Unavailable => "notice.copiedInstead",
                            });
                        }
                        Err(error) => emit(Event::State(SpeechState::Failed {
                            key: error.to_string(),
                        })),
                    }
                }
                Command::DownloadFinished { ok, error } => {
                    self.downloading = false;
                    if ok {
                        emit(Event::State(SpeechState::Idle));
                        notice("notice.modelReady");
                    } else {
                        let key = error.unwrap_or_else(|| "model.download".into());
                        emit(Event::State(SpeechState::Failed { key }));
                    }
                }
            }
        }
    }
}
