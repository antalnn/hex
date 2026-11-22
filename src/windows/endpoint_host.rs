//! Starts and stops the transcription endpoint (Labs › Transcription
//! endpoint) as the saved settings change, and keeps the model selection it
//! reads in step with the one Hex's window holds. With the lab off nothing
//! listens and no thread runs.

use std::sync::{Arc, RwLock};

use crate::app_settings::AppSettings;
use crate::local_api::openai::{self, Backend, Endpoint, StartError, Status};
use crate::local_api::processing::Processor;
use crate::transcription::models::TranscriptionSelection;

pub(crate) struct EndpointHost {
    endpoint: Option<Endpoint>,
    /// The model and language a request transcribes with, read at each
    /// request so a change in Hex's window applies at once.
    selection: Arc<RwLock<TranscriptionSelection>>,
    processor: Processor,
}

impl EndpointHost {
    pub(crate) fn new(processor: Processor, settings: &AppSettings) -> Self {
        let mut host = Self {
            endpoint: None,
            selection: Arc::new(RwLock::new(selection(settings))),
            processor,
        };
        host.apply(settings);
        host
    }

    /// Follows `settings`: the selection always, the listener by the lab. A
    /// listener that could not start (the port is taken) tries again on the
    /// next saved change.
    pub(crate) fn apply(&mut self, settings: &AppSettings) {
        *self
            .selection
            .write()
            .unwrap_or_else(|error| error.into_inner()) = selection(settings);
        let wanted = crate::labs::transcription_endpoint_enabled(&settings.labs);
        match (wanted, self.endpoint.is_some()) {
            (true, false) => self.start(),
            (false, true) => self.stop(),
            (false, false) => openai::set_status(Status::Off),
            (true, true) => {}
        }
    }

    fn start(&mut self) {
        let selection = self.selection.clone();
        let processor = self.processor.clone();
        let started = Endpoint::start(openai::PORT, |shutdown| {
            Backend::for_app(selection, processor, shutdown)
        });
        match started {
            Ok(endpoint) => {
                tracing::info!(
                    port = endpoint.port(),
                    "transcription endpoint listening on 127.0.0.1"
                );
                self.endpoint = Some(endpoint);
                openai::set_status(Status::Listening);
            }
            Err(StartError::PortInUse) => {
                tracing::warn!(
                    port = openai::PORT,
                    "transcription endpoint not started: the port is in use by another program"
                );
                openai::set_status(Status::PortInUse);
            }
            Err(error) => {
                tracing::warn!(%error, "transcription endpoint not started");
                openai::set_status(Status::Failed);
            }
        }
    }

    pub(crate) fn stop(&mut self) {
        if let Some(mut endpoint) = self.endpoint.take() {
            endpoint.stop();
        }
        openai::set_status(Status::Off);
    }
}

impl Drop for EndpointHost {
    fn drop(&mut self) {
        self.stop();
    }
}

/// The saved selection, or the default model while it names an import that
/// can't run, as the listener does.
fn selection(settings: &AppSettings) -> TranscriptionSelection {
    crate::transcription::custom_models::runnable(&settings.transcription)
}
