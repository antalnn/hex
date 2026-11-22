//! The transcription endpoint's listener: its own accept loop and workers,
//! bound to `127.0.0.1` only. It copies the shape of the local API's `serve`
//! and `http_worker` but shares nothing with that API's listener: no token,
//! no discovery file, no instance lock, no second route table.

use std::io::ErrorKind;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Instant;

use super::reply::{self, Failure};
use super::{Request, Transcribe, respond};
use crate::local_api::http::{
    BodyReadError, HttpResponse, RequestReadError, read_body_bounded, read_request, write_response,
};
use crate::local_api::{
    ACCEPT_RETRY, ACCEPT_WAIT, CONNECTION_QUEUE_CAPACITY, CONNECTION_TIMEOUT, HTTP_WORKERS,
    MAX_AUDIO_BYTES, UPLOAD_DEADLINE, wait_for_connection, wake_acceptor,
};

/// Why the endpoint did not start.
#[derive(Debug)]
pub(crate) enum StartError {
    /// Another program holds the port, such as an older bridge.
    PortInUse,
    Failed(String),
}

impl std::fmt::Display for StartError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PortInUse => formatter.write_str("the port is in use by another program"),
            Self::Failed(reason) => formatter.write_str(reason),
        }
    }
}

/// A running listener. Dropping it stops it and waits for its threads.
pub(crate) struct Endpoint {
    shutdown: Arc<AtomicBool>,
    address: SocketAddr,
    acceptor: Option<JoinHandle<()>>,
}

impl Endpoint {
    /// Binds `127.0.0.1:port` (0 picks a free port, for tests) and serves
    /// until [`Self::stop`]. `make` builds the transcriber once the port is
    /// held, from the shutdown flag that cancels work in flight, so a port
    /// that is taken costs nothing.
    pub(crate) fn start(
        port: u16,
        make: impl FnOnce(Arc<AtomicBool>) -> color_eyre::Result<Transcribe>,
    ) -> Result<Self, StartError> {
        let listener =
            TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], port))).map_err(|error| {
                if error.kind() == ErrorKind::AddrInUse {
                    StartError::PortInUse
                } else {
                    StartError::Failed(format!("could not bind the endpoint: {error}"))
                }
            })?;
        listener
            .set_nonblocking(true)
            .map_err(|error| StartError::Failed(error.to_string()))?;
        let address = listener
            .local_addr()
            .map_err(|error| StartError::Failed(error.to_string()))?;
        let port = address.port();
        let shutdown = Arc::new(AtomicBool::new(false));
        let transcribe =
            make(shutdown.clone()).map_err(|error| StartError::Failed(format!("{error:#}")))?;
        let (ready, readiness) = mpsc::sync_channel(1);
        let worker_shutdown = shutdown.clone();
        let acceptor = thread::Builder::new()
            .name("transcription-endpoint".into())
            .spawn(move || serve(listener, transcribe, worker_shutdown, ready))
            .map_err(|error| StartError::Failed(error.to_string()))?;
        match readiness.recv_timeout(std::time::Duration::from_secs(1)) {
            Ok(true) => Ok(Self {
                shutdown,
                address,
                acceptor: Some(acceptor),
            }),
            Ok(false) | Err(_) => {
                shutdown.store(true, Ordering::Release);
                wake_acceptor(port);
                let _ = acceptor.join();
                Err(StartError::Failed("the endpoint had no workers".into()))
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn address(&self) -> SocketAddr {
        self.address
    }

    pub(crate) fn port(&self) -> u16 {
        self.address.port()
    }

    pub(crate) fn stop(&mut self) {
        let Some(acceptor) = self.acceptor.take() else {
            return;
        };
        self.shutdown.store(true, Ordering::Release);
        wake_acceptor(self.port());
        if acceptor.join().is_err() {
            tracing::error!("transcription endpoint thread panicked during shutdown");
        }
        tracing::info!(port = self.port(), "transcription endpoint stopped");
    }
}

impl Drop for Endpoint {
    fn drop(&mut self) {
        self.stop();
    }
}

fn serve(
    listener: TcpListener,
    transcribe: Transcribe,
    shutdown: Arc<AtomicBool>,
    ready: mpsc::SyncSender<bool>,
) {
    let (connections, pending) = mpsc::sync_channel(CONNECTION_QUEUE_CAPACITY);
    let pending = Arc::new(Mutex::new(pending));
    let workers = (0..HTTP_WORKERS)
        .filter_map(|index| {
            let pending = pending.clone();
            let transcribe = transcribe.clone();
            let shutdown = shutdown.clone();
            thread::Builder::new()
                .name(format!("transcription-endpoint-http-{index}"))
                .spawn(move || worker(&pending, &transcribe, &shutdown))
                .map_err(|error| {
                    tracing::error!(%error, "could not start a transcription endpoint worker");
                })
                .ok()
        })
        .collect::<Vec<_>>();
    // Only the workers keep the transcriber alive from here on, so it is
    // dropped (and its service stopped) once they have all ended.
    drop(transcribe);
    if workers.is_empty() {
        let _ = ready.send(false);
        return;
    }
    if ready.send(true).is_err() {
        return;
    }
    while !shutdown.load(Ordering::Acquire) {
        match listener.accept() {
            // The connection that woke a shutdown is not a request.
            Ok(_) if shutdown.load(Ordering::Acquire) => break,
            Ok((stream, _)) => match connections.try_send(stream) {
                Ok(()) => {}
                Err(TrySendError::Full(mut stream)) => {
                    let _ = write_response(
                        &mut stream,
                        reply::error(&Failure::unavailable("Hex is busy with other requests.")),
                    );
                }
                Err(TrySendError::Disconnected(_)) => break,
            },
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                wait_for_connection(&listener, ACCEPT_WAIT);
            }
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(error) => {
                tracing::error!(%error, "transcription endpoint accept failed");
                thread::sleep(ACCEPT_RETRY);
            }
        }
    }
    drop(connections);
    for worker in workers {
        if worker.join().is_err() {
            tracing::error!("transcription endpoint worker panicked during shutdown");
        }
    }
}

fn worker(pending: &Mutex<Receiver<TcpStream>>, transcribe: &Transcribe, shutdown: &AtomicBool) {
    loop {
        let stream = pending
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .recv();
        let Ok(mut stream) = stream else {
            return;
        };
        let _ = stream.set_nodelay(true);
        if stream.set_read_timeout(Some(CONNECTION_TIMEOUT)).is_err()
            || stream.set_write_timeout(Some(CONNECTION_TIMEOUT)).is_err()
        {
            continue;
        }
        let started = Instant::now();
        let (response, request_bytes, route) = match read(&mut stream, shutdown) {
            Ok(request) => {
                let bytes = request.body.len();
                let route = format!(
                    "{} {}",
                    request.method,
                    request.path.split('?').next().unwrap_or("/")
                );
                (respond(request, transcribe), bytes, route)
            }
            Err(Some(response)) => (response, 0, "unreadable request".to_owned()),
            Err(None) => continue,
        };
        // A line per request: sizes, timing and status, never audio or text.
        tracing::info!(
            route,
            request_bytes,
            response_bytes = response.body.len(),
            status = response.status,
            elapsed_ms = started.elapsed().as_millis() as u64,
            "transcription endpoint request"
        );
        if let Err(error) = write_response(&mut stream, response) {
            tracing::debug!(%error, "transcription endpoint client disconnected");
        }
    }
}

/// Reads one request. `Err(None)` means answer nothing: Hex is shutting down
/// or the client left.
fn read(stream: &mut TcpStream, shutdown: &AtomicBool) -> Result<Request, Option<HttpResponse>> {
    let head = read_request(stream, shutdown).map_err(|error| match error {
        RequestReadError::TooLarge => Some(reply::error(&Failure::new(
            431,
            "invalid_request_error",
            "The request headers are too large.",
        ))),
        RequestReadError::Invalid => Some(reply::error(&Failure::invalid(
            "The request could not be read. Send a Content-Length body, not a chunked one.",
        ))),
        RequestReadError::Deadline => Some(reply::error(&Failure::new(
            408,
            "request_timeout",
            "The request took too long to arrive.",
        ))),
        RequestReadError::Shutdown | RequestReadError::Io(_) => None,
    })?;
    let mut body = head.body_prefix;
    if let Some(length) = head.content_length {
        read_body_bounded(
            stream,
            length,
            &mut body,
            shutdown,
            MAX_AUDIO_BYTES,
            UPLOAD_DEADLINE,
        )
        .map_err(|error| match error {
            BodyReadError::ResourceExhausted => Some(reply::error(&Failure::too_large())),
            BodyReadError::Invalid | BodyReadError::Io(_) => Some(reply::error(&Failure::invalid(
                "The request body ended early.",
            ))),
            BodyReadError::Deadline => Some(reply::error(&Failure::new(
                408,
                "request_timeout",
                "The upload took too long.",
            ))),
            BodyReadError::Shutdown => None,
        })?;
    }
    Ok(Request {
        method: head.method,
        path: head.path,
        content_type: head.content_type,
        body,
    })
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Read, Write};
    use std::net::TcpStream;
    use std::sync::atomic::AtomicUsize;
    use std::sync::mpsc;
    use std::time::Duration;

    use super::super::backend::{Backend, RunPiece, Select};
    use super::super::multipart::tests::body;
    use super::super::reply::Failure;
    use super::*;
    use crate::local_api::processing::Processor;
    use crate::transcription::models::TranscriptionSelection;

    static NEXT_FOLDER: AtomicUsize = AtomicUsize::new(0);

    /// A staging folder of this test's own, so the "nothing is kept" checks
    /// never see another test's in-flight file.
    fn folder() -> std::path::PathBuf {
        let folder = std::env::temp_dir().join(format!(
            "hex-endpoint-test-{}-{}",
            std::process::id(),
            NEXT_FOLDER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&folder);
        folder
    }

    fn files(folder: &std::path::Path) -> usize {
        std::fs::read_dir(folder).map_or(0, Iterator::count)
    }

    /// 16 kHz mono 16-bit WAV of a steady tone, `seconds` long.
    fn wav(seconds: f32) -> Vec<u8> {
        let mut bytes = Cursor::new(Vec::new());
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::new(&mut bytes, spec).unwrap();
        for index in 0..(seconds * 16_000.0) as usize {
            let sample = (index as f32 * 0.05).sin() * 8_000.0;
            writer.write_sample(sample as i16).unwrap();
        }
        writer.finalize().unwrap();
        bytes.into_inner()
    }

    fn select() -> Select {
        Box::new(|requested| {
            Ok(TranscriptionSelection {
                language: requested.unwrap_or("en").to_owned(),
                ..TranscriptionSelection::default()
            })
        })
    }

    /// Answers "piece N" and records each piece's length.
    fn pieces(seen: &Arc<Mutex<Vec<usize>>>) -> RunPiece {
        let seen = seen.clone();
        Box::new(move |_selection, samples, _limits| {
            let mut seen = seen.lock().unwrap();
            seen.push(samples.len());
            Ok(format!("piece {}", seen.len()))
        })
    }

    fn processor() -> Processor {
        Arc::new(|raw: &str, language: &str, _cancelled: &AtomicBool| {
            format!("Processed [{language}]: {raw}")
        })
    }

    fn start(
        select: Select,
        run_piece: RunPiece,
        processor: Processor,
        folder: &std::path::Path,
    ) -> Endpoint {
        let folder = folder.to_owned();
        Endpoint::start(0, move |shutdown| {
            Ok(Backend::new(select, run_piece, processor, shutdown, None)
                .in_folder(folder)
                .into_transcribe())
        })
        .unwrap()
    }

    fn send(endpoint: &Endpoint, head: &str, body: &[u8]) -> (u16, String) {
        let mut connection = TcpStream::connect(endpoint.address()).unwrap();
        connection
            .set_read_timeout(Some(Duration::from_secs(20)))
            .unwrap();
        connection.write_all(head.as_bytes()).unwrap();
        connection.write_all(body).unwrap();
        let mut response = Vec::new();
        connection.read_to_end(&mut response).unwrap();
        let response = String::from_utf8(response).unwrap();
        let status = response[9..12].parse().unwrap();
        let body = response.split_once("\r\n\r\n").unwrap().1.to_owned();
        (status, body)
    }

    fn post(endpoint: &Endpoint, fields: &[(&str, &str)], audio: &[u8]) -> (u16, String) {
        let body = body("xyz", fields, audio);
        send(
            endpoint,
            &format!(
                "POST /v1/audio/transcriptions HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: multipart/form-data; boundary=xyz\r\nContent-Length: {}\r\n\r\n",
                body.len()
            ),
            &body,
        )
    }

    #[test]
    fn the_listener_binds_loopback_only() {
        let folder = folder();
        let endpoint = start(select(), pieces(&Arc::default()), processor(), &folder);
        assert_eq!(
            endpoint.address().ip(),
            std::net::IpAddr::from([127, 0, 0, 1])
        );
    }

    #[test]
    fn a_real_upload_comes_back_as_the_pipeline_text_not_the_raw_transcript() {
        let folder = folder();
        let seen = Arc::default();
        let endpoint = start(select(), pieces(&seen), processor(), &folder);
        let (status, reply) = post(
            &endpoint,
            &[
                ("model", "hex"),
                ("response_format", "json"),
                ("language", "auto"),
            ],
            &wav(2.0),
        );
        assert_eq!(status, 200, "{reply}");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&reply).unwrap(),
            serde_json::json!({ "text": "Processed [en]: piece 1" })
        );
        assert_eq!(*seen.lock().unwrap(), vec![32_000]);
        assert_eq!(files(&folder), 0, "the staged upload is deleted");
    }

    #[test]
    fn a_requested_language_reaches_the_selection_and_the_processor() {
        let folder = folder();
        let endpoint = start(select(), pieces(&Arc::default()), processor(), &folder);
        let (status, reply) = post(&endpoint, &[("language", "sv")], &wav(1.0));
        assert_eq!(status, 200, "{reply}");
        assert!(reply.contains("Processed [sv]"), "{reply}");
    }

    #[test]
    fn audio_over_30_seconds_is_transcribed_as_two_pieces() {
        let folder = folder();
        let seen = Arc::default();
        let endpoint = start(select(), pieces(&seen), processor(), &folder);
        let (status, reply) = post(
            &endpoint,
            &[("response_format", "verbose_json")],
            &wav(31.0),
        );
        assert_eq!(status, 200, "{reply}");
        let reply: serde_json::Value = serde_json::from_str(&reply).unwrap();
        assert_eq!(reply["text"], "Processed [en]: piece 1 piece 2");
        assert_eq!(reply["duration"], 31.0);
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 2, "{seen:?}");
        assert!(seen.iter().all(|length| *length <= 30 * 16_000), "{seen:?}");
        assert_eq!(seen.iter().sum::<usize>(), 31 * 16_000);
    }

    #[test]
    fn audio_that_cannot_be_decoded_is_refused_and_deleted() {
        let folder = folder();
        let seen = Arc::default();
        let endpoint = start(select(), pieces(&seen), processor(), &folder);
        let (status, reply) = post(&endpoint, &[], b"this is not audio at all, just text");
        assert_eq!(status, 400, "{reply}");
        assert!(reply.contains("invalid_request_error"), "{reply}");
        assert!(seen.lock().unwrap().is_empty());
        assert_eq!(files(&folder), 0);
    }

    #[test]
    fn silence_answers_an_empty_text_without_running_the_pipeline() {
        let folder = folder();
        let endpoint = start(
            select(),
            Box::new(|_, _, _| Ok(String::new())),
            Arc::new(|_: &str, _: &str, _: &AtomicBool| -> String { panic!("nothing to process") }),
            &folder,
        );
        let (status, reply) = post(&endpoint, &[], &wav(1.0));
        assert_eq!(status, 200, "{reply}");
        assert_eq!(reply, r#"{"text":""}"#);
    }

    #[test]
    fn a_model_that_is_not_ready_answers_503_before_any_audio_is_staged() {
        let folder = folder();
        let endpoint = start(
            Box::new(|_| {
                Err(Failure::unavailable(
                    "Hex's speech model is not downloaded yet.",
                ))
            }),
            pieces(&Arc::default()),
            processor(),
            &folder,
        );
        let (status, reply) = post(&endpoint, &[], &wav(1.0));
        assert_eq!(status, 503, "{reply}");
        assert!(reply.contains("hex_unavailable"), "{reply}");
        assert_eq!(files(&folder), 0);
    }

    #[test]
    fn a_second_request_while_one_runs_gets_429() {
        let folder = folder();
        let (entered, running) = mpsc::channel();
        let (release, held) = mpsc::channel::<()>();
        let held = Mutex::new(held);
        let processor: Processor = Arc::new(move |raw: &str, _: &str, _: &AtomicBool| {
            entered.send(()).unwrap();
            held.lock().unwrap().recv().unwrap();
            raw.to_owned()
        });
        let endpoint = Arc::new(start(select(), pieces(&Arc::default()), processor, &folder));
        let first = {
            let endpoint = endpoint.clone();
            thread::spawn(move || post(&endpoint, &[], &wav(1.0)))
        };
        running.recv_timeout(Duration::from_secs(20)).unwrap();
        let (status, reply) = post(&endpoint, &[], &wav(1.0));
        assert_eq!(status, 429, "{reply}");
        assert!(reply.contains("hex_busy"), "{reply}");
        release.send(()).unwrap();
        let (status, reply) = first.join().unwrap();
        assert_eq!(status, 200, "{reply}");
        // The slot is free again.
        release.send(()).unwrap();
        let (status, _) = post(&endpoint, &[], &wav(1.0));
        assert_eq!(status, 200);
    }

    #[test]
    fn health_and_unknown_routes_answer_over_tcp() {
        let folder = folder();
        let endpoint = start(select(), pieces(&Arc::default()), processor(), &folder);
        let (status, reply) = send(
            &endpoint,
            "GET /v1/health HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n",
            b"",
        );
        assert_eq!(status, 200);
        assert!(reply.contains(r#""ok":true"#), "{reply}");
        let (status, _) = send(&endpoint, "GET /nope HTTP/1.1\r\n\r\n", b"");
        assert_eq!(status, 404);
        // A chunked upload is refused, not misread.
        let (status, _) = send(
            &endpoint,
            "POST /v1/audio/transcriptions HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n",
            b"0\r\n\r\n",
        );
        assert_eq!(status, 400);
    }

    #[test]
    fn an_oversized_declared_upload_is_refused_unread() {
        let folder = folder();
        let endpoint = start(select(), pieces(&Arc::default()), processor(), &folder);
        let head = format!(
            "POST /v1/audio/transcriptions HTTP/1.1\r\nContent-Type: multipart/form-data; boundary=x\r\nContent-Length: {}\r\n\r\n",
            MAX_AUDIO_BYTES + 1
        );
        let (status, reply) = send(&endpoint, &head, b"");
        assert_eq!(status, 413, "{reply}");
    }

    #[test]
    fn nothing_listens_after_stop() {
        let folder = folder();
        let mut endpoint = start(select(), pieces(&Arc::default()), processor(), &folder);
        let address = endpoint.address();
        assert!(TcpStream::connect(address).is_ok());
        endpoint.stop();
        assert!(
            TcpStream::connect_timeout(&address, Duration::from_millis(500)).is_err(),
            "the port still answers after stop"
        );
        // Stopping again, and dropping, are harmless.
        endpoint.stop();
    }

    #[test]
    fn a_port_in_use_is_reported_and_builds_nothing() {
        let holder = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = holder.local_addr().unwrap().port();
        let built = AtomicBool::new(false);
        let result = Endpoint::start(port, |_| {
            built.store(true, Ordering::Release);
            Err(color_eyre::eyre::eyre!("not reached"))
        });
        assert!(matches!(result, Err(StartError::PortInUse)));
        assert!(!built.load(Ordering::Acquire));
    }
}
