//! The local llama.cpp server that backs the local rewrite transformation.
//!
//! Hex starts one child (`llama-server`) and owns it: it is started when the
//! app opens, its health is polled until it answers, and it is killed when Hex
//! quits, so the console window a person would otherwise leave open is never
//! needed. A server already answering the port is adopted instead of fought:
//! two servers cannot share one port, and the one that is already there is
//! the one the transformation talks to.

use std::path::PathBuf;
use std::process::{Child, Stdio};
use std::time::{Duration, Instant};

/// How long a freshly started server has to answer `/health`.
const READY_TIMEOUT: Duration = Duration::from_secs(45);
/// One health probe's budget. A server that is loading a model keeps the
/// connection open, so this is short on purpose: the next poll comes soon.
const PROBE_TIMEOUT: Duration = Duration::from_millis(600);
/// The runtime ticks faster than this; a health probe is not worth one per
/// tick, so probes are spaced out.
const PROBE_INTERVAL: Duration = Duration::from_secs(1);

/// The model this build starts. The file lives in Hex's own support folder, so
/// uninstalling the app that used to serve it cannot break Hex.
pub(crate) const DEFAULT_MODEL: &str = "fluid-1-mini-q6_k.gguf";
/// The port the personal `local-rewrite` transformation already calls.
pub(crate) const DEFAULT_PORT: u16 = 8081;

/// The generation flags the transformation was tuned with: the 4096-token
/// context, all layers on the GPU, chat templating, and n-gram speculative
/// decoding, which is what keeps a rewrite near 200 ms.
pub(crate) const DEFAULT_ARGS: &[&str] = &[
    "-c",
    "4096",
    "-ngl",
    "99",
    "--jinja",
    "-rea",
    "off",
    "--temp",
    "0",
    "--spec-type",
    "ngram-simple",
    "--spec-ngram-simple-size-n",
    "3",
    "--spec-ngram-simple-size-m",
    "16",
];

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) enum State {
    /// No server Hex owns, and nothing answers the port.
    #[default]
    Stopped,
    /// Hex started a child; it has not answered yet.
    Starting,
    /// Hex's own child answers. It is killed when Hex quits.
    Ready,
    /// A server Hex did not start answers; it is used and left alone.
    External,
    /// The child could not start, exited, or never answered.
    Failed,
    /// No llama-server program is on this computer, so nothing was started.
    Missing,
}

impl State {
    /// The words the sidebar footer shows for this state.
    pub(crate) fn words(self) -> &'static str {
        match self {
            Self::Stopped => "off",
            Self::Starting => "starting",
            Self::Ready => "running",
            Self::External => "running (external)",
            Self::Failed => "failed",
            Self::Missing => "not installed",
        }
    }
}

/// What the window shows about the server.
#[derive(Clone, Debug, Default, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Snapshot {
    pub(crate) state: State,
    /// Why it failed, or where an external server came from.
    #[serde(default)]
    pub(crate) detail: Option<String>,
}

/// Everything needed to start the server. Hardcoded defaults until the
/// settings exist; each field is what a setting will replace.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Config {
    pub program: PathBuf,
    pub model: PathBuf,
    pub port: u16,
    pub args: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        let program = resolve_program(&program_candidates(None));
        // Hex's own models folder (`%LOCALAPPDATA%\hex\models` on Windows,
        // which this computer's Hex migrates its models into), so the model
        // never depends on another app's install.
        let model = crate::app_paths::models_dir()
            .map(|dir| dir.join("local-llm").join(DEFAULT_MODEL))
            .unwrap_or_else(|_| PathBuf::from(DEFAULT_MODEL));
        Self {
            program,
            model,
            port: DEFAULT_PORT,
            args: DEFAULT_ARGS.iter().map(|arg| (*arg).to_string()).collect(),
        }
    }
}

/// The `llama-server` file name this computer runs.
fn program_name() -> &'static str {
    if cfg!(windows) {
        "llama-server.exe"
    } else {
        "llama-server"
    }
}

/// Where a llama.cpp copy shipped beside Hex's own model lives: the folder a
/// portable install unpacks into.
fn bundled_program() -> Option<PathBuf> {
    crate::app_paths::models_dir()
        .ok()
        .map(|dir| dir.join("local-llm").join("llama.cpp").join(program_name()))
}

/// The places `llama-server` may be, in the order they are tried: an
/// environment override, the configured path, a copy bundled beside the
/// model, then the home-folder install.
fn program_candidates(configured: Option<PathBuf>) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(program) = std::env::var_os("HEX_LLAMA_SERVER") {
        candidates.push(PathBuf::from(program));
    }
    if let Some(program) = configured {
        candidates.push(program);
    }
    if let Some(program) = bundled_program() {
        candidates.push(program);
    }
    candidates.push(
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("tools")
            .join("llama-cpp")
            .join(program_name()),
    );
    candidates
}

/// The first candidate that exists, so an installed copy is found wherever it
/// is; when none does, the first named one, so the message names a path a
/// person can create.
fn resolve_program(candidates: &[PathBuf]) -> PathBuf {
    candidates
        .iter()
        .find(|candidate| candidate.is_file())
        .or_else(|| candidates.first())
        .cloned()
        .unwrap_or_else(|| PathBuf::from(program_name()))
}

impl Config {
    /// The child's arguments, in order: the model, then the port, then the
    /// generation flags.
    pub(crate) fn argv(&self) -> Vec<String> {
        let mut argv = vec![
            "-m".to_string(),
            self.model.display().to_string(),
            "--port".to_string(),
            self.port.to_string(),
        ];
        argv.extend(self.args.iter().cloned());
        argv
    }

    fn health_url(&self) -> String {
        format!("http://127.0.0.1:{}/health", self.port)
    }

    /// The settings' values, with the built-in defaults filling anything left
    /// empty.
    pub(crate) fn from_settings(llm: &crate::app_settings::LlmSettings) -> Self {
        let defaults = Self::default();
        Self {
            program: resolve_program(&program_candidates(llm.program.clone())),
            model: llm.model.clone().unwrap_or(defaults.model),
            port: llm.port,
            args: if llm.args.is_empty() {
                defaults.args
            } else {
                llm.args.clone()
            },
        }
    }
}

/// The one server Hex owns, if it owns one.
pub(crate) struct LocalModel {
    config: Config,
    child: Option<Child>,
    state: State,
    detail: Option<String>,
    started_at: Option<Instant>,
    last_probe: Option<Instant>,
}

impl LocalModel {
    pub(crate) fn new(config: Config) -> Self {
        Self {
            config,
            child: None,
            state: State::Stopped,
            detail: None,
            started_at: None,
            last_probe: None,
        }
    }

    pub(crate) fn snapshot(&self) -> Snapshot {
        Snapshot {
            state: self.state,
            detail: self.detail.clone(),
        }
    }

    /// Starts the child unless a server already answers, or one Hex started is
    /// still alive. A second start is what the Output button does when the
    /// state says off, so it must be safe to call at any time.
    pub(crate) fn start(&mut self) {
        if self.child.is_some() || matches!(self.state, State::Ready | State::External) {
            return;
        }
        if answering(&self.config) {
            self.state = State::External;
            self.detail = Some(format!("already answering on port {}", self.config.port));
            self.started_at = None;
            return;
        }
        if !self.config.program.is_file() {
            self.missing(format!(
                "llama-server was not found at {}",
                self.config.program.display()
            ));
            return;
        }
        if !self.config.model.is_file() {
            self.fail(format!(
                "the model file was not found at {}",
                self.config.model.display()
            ));
            return;
        }
        let mut command = crate::desktop::background_process::command(&self.config.program);
        command
            .args(self.config.argv())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        match command.spawn() {
            Ok(child) => {
                tracing::info!(
                    program = %self.config.program.display(),
                    model = %self.config.model.display(),
                    port = self.config.port,
                    "starting the local model server"
                );
                self.child = Some(child);
                self.state = State::Starting;
                self.detail = None;
                self.started_at = Some(Instant::now());
            }
            Err(error) => self.fail(format!("could not start llama-server: {error}")),
        }
    }

    /// Moves a starting server to ready, notices a child that died, and
    /// notices an external server that went away. Cheap enough to call on
    /// every window poll: a ready server costs one local request.
    pub(crate) fn poll(&mut self) {
        match self.state {
            State::Starting => {
                if self.probe_due() && answering(&self.config) {
                    self.state = State::Ready;
                    self.detail = None;
                    self.started_at = None;
                    tracing::info!(port = self.config.port, "the local model server is ready");
                    return;
                }
                if let Some(status) = self.child_exit() {
                    self.fail(format!("llama-server exited during startup ({status})"));
                    return;
                }
                if self
                    .started_at
                    .is_some_and(|started| started.elapsed() > READY_TIMEOUT)
                {
                    self.kill();
                    self.fail("llama-server did not answer within 45 seconds".into());
                }
            }
            State::Ready => {
                if let Some(status) = self.child_exit() {
                    self.fail(format!("llama-server exited ({status})"));
                }
            }
            State::External => {
                if self.probe_due() && !answering(&self.config) {
                    self.state = State::Stopped;
                    self.detail = None;
                }
            }
            State::Stopped | State::Failed | State::Missing => {}
        }
    }

    /// Whether a health probe is due now, spacing probes out so a fast
    /// runtime tick does not flood the server.
    fn probe_due(&mut self) -> bool {
        let now = Instant::now();
        if self
            .last_probe
            .is_some_and(|last| now.duration_since(last) < PROBE_INTERVAL)
        {
            return false;
        }
        self.last_probe = Some(now);
        true
    }

    /// Stops the child Hex owns. An external server is left running.
    pub(crate) fn stop(&mut self) {
        self.kill();
        self.state = State::Stopped;
        self.detail = None;
        self.started_at = None;
    }

    fn fail(&mut self, detail: String) {
        tracing::warn!(%detail, "the local model server is not running");
        self.kill();
        self.state = State::Failed;
        self.detail = Some(detail);
        self.started_at = None;
    }

    /// Nothing tried to start: no llama-server program is installed. The
    /// local model stays off until one appears, and nothing logs this as an
    /// error.
    fn missing(&mut self, detail: String) {
        tracing::info!(%detail, "the local model server needs llama.cpp installed");
        self.kill();
        self.state = State::Missing;
        self.detail = Some(detail);
        self.started_at = None;
    }

    /// The exit status of a child that has already finished, if it has.
    fn child_exit(&mut self) -> Option<std::process::ExitStatus> {
        let child = self.child.as_mut()?;
        match child.try_wait() {
            Ok(Some(status)) => Some(status),
            _ => None,
        }
    }

    fn kill(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for LocalModel {
    fn drop(&mut self) {
        self.kill();
    }
}

/// Whether a llama server answers this config's port. The body is checked too,
/// so a different program squatting on the port is not mistaken for the model
/// server: llama.cpp's `/health` reports `{"status":"ok"}`.
fn answering(config: &Config) -> bool {
    let agent = crate::net::loopback_agent(|settings| settings.timeout_global(Some(PROBE_TIMEOUT)));
    let Ok(mut response) = agent.get(&config.health_url()).call() else {
        return false;
    };
    if !response.status().is_success() {
        return false;
    }
    let body = response
        .body_mut()
        .with_config()
        .limit(256)
        .read_to_vec()
        .unwrap_or_default();
    String::from_utf8_lossy(&body).contains("ok")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_server_is_stopped() {
        let model = LocalModel::new(Config::default());
        assert_eq!(model.snapshot().state, State::Stopped);
        assert_eq!(model.snapshot().state.words(), "off");
    }

    #[test]
    fn the_arguments_carry_the_model_and_the_port() {
        let config = Config::default();
        let argv = config.argv();
        assert_eq!(argv[0], "-m");
        assert_eq!(argv[1], config.model.display().to_string());
        assert_eq!(argv[2], "--port");
        assert_eq!(argv[3], "8081");
        assert!(argv.iter().any(|arg| arg == "-ngl"));
        assert!(argv.iter().any(|arg| arg == "--jinja"));
        assert!(config.health_url().ends_with(":8081/health"));
    }

    #[test]
    fn the_default_model_lives_in_hexs_own_folder() {
        let config = Config::default();
        let text = config.model.display().to_string();
        assert!(text.contains("hex"), "{text}");
        assert!(text.ends_with(DEFAULT_MODEL), "{text}");
    }

    #[test]
    fn a_port_nothing_listens_on_is_not_answering() {
        let config = Config {
            port: 9,
            ..Config::default()
        };
        assert!(!answering(&config));
    }

    #[test]
    fn stopping_clears_the_child() {
        let mut model = LocalModel::new(Config::default());
        model.state = State::Ready;
        model.stop();
        assert_eq!(model.snapshot().state, State::Stopped);
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("hex-local-model-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_first_program_that_exists_wins() {
        let dir = temp_dir("resolve");
        let home = dir.join("home").join(program_name());
        let bundled = dir.join("bundle").join(program_name());
        std::fs::create_dir_all(bundled.parent().unwrap()).unwrap();
        std::fs::write(&bundled, b"program").unwrap();
        assert_eq!(
            resolve_program(&[home.clone(), bundled.clone()]),
            bundled,
            "an existing copy is found past a candidate that is absent"
        );
        assert_eq!(
            resolve_program(std::slice::from_ref(&home)),
            home,
            "with nothing installed the message names the first candidate"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn candidates_read_environment_then_settings_then_bundle_then_home() {
        let dir = temp_dir("candidates");
        let configured = dir.join("chosen").join(program_name());
        let candidates = program_candidates(Some(configured.clone()));
        let configured_at = candidates
            .iter()
            .position(|candidate| *candidate == configured)
            .expect("the configured path is a candidate");
        if std::env::var_os("HEX_LLAMA_SERVER").is_some() {
            assert_eq!(configured_at, 1, "the environment override comes first");
        } else {
            assert_eq!(configured_at, 0, "the configured path comes first");
        }
        assert!(
            candidates.last().is_some_and(|candidate| candidate
                .parent()
                .is_some_and(|parent| parent.ends_with("llama-cpp"))),
            "the home-folder install is the last candidate: {candidates:?}"
        );
        // The bundled copy, when Hex has a models folder, sits just before
        // the home-folder install.
        let bundled_at = candidates.iter().position(|candidate| {
            candidate
                .parent()
                .is_some_and(|parent| parent.ends_with("llama.cpp"))
        });
        if let Some(at) = bundled_at {
            assert_eq!(
                at,
                candidates.len() - 2,
                "the bundled copy comes before the home install: {candidates:?}"
            );
            assert!(candidates[at].ends_with(program_name()));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_program_leaves_the_local_model_off() {
        let dir = temp_dir("missing");
        let config = Config {
            program: dir.join("absent").join(program_name()),
            model: dir.join("model.gguf"),
            port: 9,
            args: Vec::new(),
        };
        let mut model = LocalModel::new(config);
        model.start();
        let snapshot = model.snapshot();
        assert_eq!(snapshot.state, State::Missing);
        assert_eq!(snapshot.state.words(), "not installed");
        assert!(
            snapshot
                .detail
                .as_deref()
                .is_some_and(|detail| detail.contains("llama-server")),
            "{snapshot:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
