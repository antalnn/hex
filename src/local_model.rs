//! The local llama.cpp server the dictation cleanup calls, and one job for the
//! personal `local-rewrite` transformation.
//!
//! Hex starts one child (`llama-server`) and owns it: it is started when the
//! app opens, its health is polled until it answers, and it dies whenever Hex
//! ends, however it ends: a Windows job object kills it when Hex's process
//! closes the handle, including on a crash or a force kill. A child that exits
//! while Hex is running is started again, at most three times in five minutes,
//! and its stderr is kept in `logs/llama-server.log`. A server Hex recorded at
//! spawn is its own orphan from an earlier run and is stopped so the GPU memory
//! comes back; any other server on the preferred port is left alone, and Hex
//! moves its child to a free loopback port.

#[cfg(windows)]
use std::fs;
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
/// A child that exits while Hex is running is started again after this pause.
const RESTART_DELAY: Duration = Duration::from_secs(2);
/// No more than this many restarts in [`RESTART_WINDOW`], so a server that
/// cannot start does not loop forever.
const RESTART_LIMIT: u32 = 3;
const RESTART_WINDOW: Duration = Duration::from_secs(300);
/// A server that stays ready this long clears the restart count.
const RESTART_RESET_AFTER: Duration = Duration::from_secs(60);

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
    /// No server Hex owns.
    #[default]
    Stopped,
    /// Hex started a child; it has not answered yet.
    Starting,
    /// Hex's own child answers. It is killed when Hex quits.
    Ready,
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
            Self::Failed => "failed",
            Self::Missing => "not installed",
        }
    }
}

/// What the window shows about the server.
#[derive(Clone, Debug, Default, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Snapshot {
    pub(crate) state: State,
    /// Why it failed, when it did.
    #[serde(default)]
    pub(crate) detail: Option<String>,
    /// The loopback port the child listens on. The preferred port while
    /// stopped; a fallback when the preferred one was busy.
    #[serde(default)]
    pub(crate) port: u16,
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
    /// The same arguments with an actual port, which may differ from the
    /// preferred one when it was busy.
    pub(crate) fn argv_for(&self, port: u16) -> Vec<String> {
        let mut argv = vec![
            "-m".to_string(),
            self.model.display().to_string(),
            "--port".to_string(),
            port.to_string(),
        ];
        argv.extend(self.args.iter().cloned());
        argv
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
    /// The loopback port the current child uses. Starts as the preferred one.
    port: u16,
    child: Option<Child>,
    /// Windows: ends the child when Hex's process ends, even without Drop.
    #[cfg(windows)]
    job: Option<crate::personal_commands::process::windows_job::Job>,
    state: State,
    detail: Option<String>,
    started_at: Option<Instant>,
    last_probe: Option<Instant>,
    /// When an exited child may be started again.
    restart_at: Option<Instant>,
    /// The first restart inside the current window, and how many followed.
    restart_window: Option<Instant>,
    restarts: u32,
    /// When the server last became ready, so a stable run clears the count.
    ready_at: Option<Instant>,
}

impl LocalModel {
    pub(crate) fn new(config: Config) -> Self {
        Self {
            port: config.port,
            config,
            child: None,
            #[cfg(windows)]
            job: None,
            state: State::Stopped,
            detail: None,
            started_at: None,
            last_probe: None,
            restart_at: None,
            restart_window: None,
            restarts: 0,
            ready_at: None,
        }
    }

    pub(crate) fn snapshot(&self) -> Snapshot {
        Snapshot {
            state: self.state,
            detail: self.detail.clone(),
            port: self.port,
        }
    }

    /// Starts the child unless Hex already owns a live one. A busy preferred
    /// port is left alone; the child moves to a free loopback port. A second
    /// start is what the Output button does when the
    /// state says off, so it must be safe to call at any time.
    pub(crate) fn start(&mut self) {
        if self.child.is_some() || self.state == State::Ready {
            return;
        }
        self.restart_at = None;
        self.ready_at = None;
        // A server Hex left behind on a previous run is Hex's own, not one a
        // person started: stop it so the preferred port comes back.
        self.kill_orphan();
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
        let preferred = self.config.port;
        self.port = choose_port(preferred);
        if self.port != preferred {
            tracing::info!(
                preferred,
                port = self.port,
                "the preferred local model port is busy; using another loopback port"
            );
        }
        // Every child goes into a job that kills it when Hex's process ends,
        // so a crash or a force kill cannot leave it holding GPU memory.
        #[cfg(windows)]
        let job = match crate::personal_commands::process::windows_job::Job::new() {
            Ok(job) => job,
            Err(error) => {
                self.fail(format!("could not tie llama-server to Hex: {error}"));
                return;
            }
        };
        let mut command = crate::desktop::background_process::command(&self.config.program);
        // The server's own log is the only place a startup failure explains
        // itself; the app's log saw only "exited (exit code: 1)".
        let log = server_log();
        let log_path = log.as_ref().map(|(_, path)| path.clone());
        let (stdout, stderr) = match log {
            Some((file, _)) => match file.try_clone() {
                Ok(copy) => (Stdio::from(file), Stdio::from(copy)),
                Err(error) => {
                    tracing::warn!(%error, "could not open the local model server's log");
                    (Stdio::null(), Stdio::null())
                }
            },
            None => (Stdio::null(), Stdio::null()),
        };
        command
            .args(self.config.argv_for(self.port))
            .stdin(Stdio::null())
            .stdout(stdout)
            .stderr(stderr);
        match command.spawn() {
            Ok(mut child) => {
                #[cfg(windows)]
                if let Err(error) = job.assign(&child) {
                    let _ = child.kill();
                    let _ = child.wait();
                    self.fail(format!("could not tie llama-server to Hex: {error}"));
                    return;
                }
                let pid = child.id();
                tracing::info!(
                    program = %self.config.program.display(),
                    model = %self.config.model.display(),
                    port = self.port,
                    preferred,
                    pid,
                    log = log_path
                        .as_ref()
                        .map(|path| path.display().to_string())
                        .as_deref()
                        .unwrap_or("none"),
                    "starting the local model server"
                );
                write_pid(pid);
                #[cfg(windows)]
                {
                    self.job = Some(job);
                }
                self.child = Some(child);
                self.state = State::Starting;
                self.detail = None;
                self.started_at = Some(Instant::now());
            }
            Err(error) => self.fail(format!("could not start llama-server: {error}")),
        }
    }

    /// Moves a starting server to ready and notices a child that died. Cheap
    /// enough to call on every window poll: a ready server costs one local
    /// request.
    pub(crate) fn poll(&mut self) {
        if self.restart_at.is_some_and(|due| Instant::now() >= due) {
            self.restart_at = None;
            self.start();
        }
        match self.state {
            State::Starting => {
                if self.probe_due() && answering(self.port) {
                    self.state = State::Ready;
                    self.detail = None;
                    self.started_at = None;
                    self.ready_at = Some(Instant::now());
                    tracing::info!(port = self.port, "the local model server is ready");
                    return;
                }
                if let Some(status) = self.child_exit() {
                    self.restart_after_exit(format!(
                        "llama-server exited during startup ({status})"
                    ));
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
                    self.restart_after_exit(format!("llama-server exited ({status})"));
                    return;
                }
                // A run this stable is not a restart loop; a later exit starts
                // the count over.
                if self.restarts > 0
                    && self
                        .ready_at
                        .is_some_and(|ready| ready.elapsed() > RESTART_RESET_AFTER)
                {
                    self.restarts = 0;
                    self.restart_window = None;
                }
            }
            State::Stopped | State::Failed | State::Missing => {}
        }
    }

    /// A child exited while Hex still owns it: start another one after a short
    /// pause, with a bound so a server that cannot stay up does not loop. The
    /// stderr log named in the start line holds the child's own reason.
    fn restart_after_exit(&mut self, detail: String) {
        self.kill();
        self.ready_at = None;
        let now = Instant::now();
        if self
            .restart_window
            .is_none_or(|start| now.duration_since(start) > RESTART_WINDOW)
        {
            self.restart_window = Some(now);
            self.restarts = 0;
        }
        self.restarts += 1;
        if self.restarts > RESTART_LIMIT {
            self.fail(format!(
                "{detail}; it keeps exiting, so see logs/llama-server.log"
            ));
            return;
        }
        tracing::warn!(
            %detail,
            attempt = self.restarts,
            "the local model server exited; restarting it"
        );
        self.state = State::Starting;
        self.detail = Some(detail);
        self.started_at = None;
        self.restart_at = Some(now + RESTART_DELAY);
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

    /// Stops the child Hex owns. Nothing else on the port is touched.
    pub(crate) fn stop(&mut self) {
        self.kill();
        self.state = State::Stopped;
        self.detail = None;
        self.started_at = None;
        self.restart_at = None;
        self.ready_at = None;
        self.restarts = 0;
        self.restart_window = None;
        self.port = self.config.port;
    }

    /// The loopback port the cleanup should call: the one this child uses,
    /// which is the preferred port until a busy one moves it.
    pub(crate) fn port(&self) -> u16 {
        self.port
    }

    fn fail(&mut self, detail: String) {
        tracing::warn!(%detail, "the local model server is not running");
        self.kill();
        self.state = State::Failed;
        self.detail = Some(detail);
        self.started_at = None;
        self.restart_at = None;
        self.ready_at = None;
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
        self.restart_at = None;
        self.ready_at = None;
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
        let owned = self.child.is_some();
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if owned {
            clear_pid();
        }
        #[cfg(windows)]
        {
            self.job = None;
        }
    }

    /// A pid Hex recorded at spawn and left behind means the server answering
    /// the port is Hex's own orphan: stop it so the GPU memory comes back.
    /// A server Hex never recorded is left alone; [`Self::start`] moves the
    /// child to a free loopback port instead.
    fn kill_orphan(&mut self) {
        #[cfg(windows)]
        if let Some(path) = state_path() {
            self.kill_orphan_at(&path);
        }
    }

    #[cfg(windows)]
    fn kill_orphan_at(&mut self, path: &std::path::Path) {
        let Some(pid) = read_pid_at(path) else {
            return;
        };
        if pid != 0
            && pid != std::process::id()
            && process_alive(pid)
            && process_name(pid).is_some_and(|name| is_llama_server(&name))
        {
            tracing::info!(
                pid,
                "stopping the local model server a previous Hex left behind"
            );
            kill_pid(pid);
            // Wait for the killed orphan to release the port before the next
            // start chooses it.
            for _ in 0..40 {
                if !process_alive(pid) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
        let _ = fs::remove_file(path);
    }
}

/// Where Hex records its child's pid across a run that ends without Drop.
#[cfg(windows)]
fn state_path() -> Option<PathBuf> {
    crate::app_paths::support_dir()
        .ok()
        .map(|dir| dir.join("local-model.pid"))
}

/// Opens `logs/llama-server.log` for the child's own stdout and stderr, with a
/// start marker so consecutive runs stay apart. `None` leaves the handles
/// null as before.
fn server_log() -> Option<(std::fs::File, PathBuf)> {
    use std::io::Write;

    let path = crate::app_paths::logs_dir().ok()?.join("llama-server.log");
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .ok()?;
    let _ = writeln!(file, "\n===== start =====");
    Some((file, path))
}

#[cfg(windows)]
fn read_pid_at(path: &std::path::Path) -> Option<u32> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

#[cfg(windows)]
fn write_pid(pid: u32) {
    let Some(path) = state_path() else { return };
    if let Err(error) = fs::write(&path, pid.to_string()) {
        tracing::warn!(%error, "could not record the local model server's pid");
    }
}

#[cfg(not(windows))]
fn write_pid(_pid: u32) {}

#[cfg(windows)]
fn clear_pid() {
    if let Some(path) = state_path() {
        let _ = fs::remove_file(path);
    }
}

#[cfg(not(windows))]
fn clear_pid() {}

/// Whether `pid` names a running process.
#[cfg(windows)]
fn process_alive(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    // SAFETY: the handle is closed before returning; a null handle is checked.
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return false;
        }
        let mut code = 0;
        let read = GetExitCodeProcess(handle, &mut code);
        CloseHandle(handle);
        read != 0 && code == STILL_ACTIVE as u32
    }
}

/// Ends `pid`, which [`process_alive`] reported.
#[cfg(windows)]
fn kill_pid(pid: u32) {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_TERMINATE, TerminateProcess};
    // SAFETY: the handle is closed before returning; a null handle is checked.
    unsafe {
        let handle = OpenProcess(PROCESS_TERMINATE, 0, pid);
        if handle.is_null() {
            return;
        }
        TerminateProcess(handle, 1);
        CloseHandle(handle);
    }
}

/// The full path of `pid`'s executable, when it can be read.
#[cfg(windows)]
fn process_name(pid: u32) -> Option<String> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
    };
    // SAFETY: the handle is closed before returning; the buffer is sized by
    // the call and read only as far as it reports.
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return None;
        }
        let mut buffer = [0u16; 1024];
        let mut size = buffer.len() as u32;
        let read = QueryFullProcessImageNameW(handle, 0, buffer.as_mut_ptr(), &mut size);
        CloseHandle(handle);
        (read != 0).then(|| String::from_utf16_lossy(&buffer[..size as usize]))
    }
}

/// Whether `path` names the server Hex starts, so a reused pid is never killed.
#[cfg(windows)]
fn is_llama_server(path: &str) -> bool {
    std::path::Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("llama-server.exe"))
}

impl Drop for LocalModel {
    fn drop(&mut self) {
        self.kill();
    }
}

/// Whether a llama server answers this config's port. The body is checked too,
/// so a different program squatting on the port is not mistaken for the model
/// server: llama.cpp's `/health` reports `{"status":"ok"}`.
fn answering(port: u16) -> bool {
    let agent = crate::net::loopback_agent(|settings| settings.timeout_global(Some(PROBE_TIMEOUT)));
    let Ok(mut response) = agent.get(&health_url(port)).call() else {
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

fn health_url(port: u16) -> String {
    format!("http://127.0.0.1:{port}/health")
}

/// The preferred port when it is free, otherwise a port the OS assigns on
/// loopback. The bind is dropped before the child starts; a foreign process
/// winning the race shows up as a startup exit and the supervisor tries again.
fn choose_port(preferred: u16) -> u16 {
    if port_is_free(preferred) {
        return preferred;
    }
    free_loopback_port().unwrap_or(preferred)
}

fn port_is_free(port: u16) -> bool {
    std::net::TcpListener::bind(("127.0.0.1", port)).is_ok()
}

fn free_loopback_port() -> Option<u16> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).ok()?;
    listener.local_addr().ok().map(|address| address.port())
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
        let argv = config.argv_for(config.port);
        assert_eq!(argv[0], "-m");
        assert_eq!(argv[1], config.model.display().to_string());
        assert_eq!(argv[2], "--port");
        assert_eq!(argv[3], "8081");
        assert!(argv.iter().any(|arg| arg == "-ngl"));
        assert!(argv.iter().any(|arg| arg == "--jinja"));
        assert!(health_url(config.port).ends_with(":8081/health"));

        let fallback = config.argv_for(8082);
        assert_eq!(fallback[2], "--port");
        assert_eq!(fallback[3], "8082");
    }

    #[test]
    fn a_free_preferred_port_is_kept() {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let free = listener.local_addr().unwrap().port();
        drop(listener);
        assert_eq!(choose_port(free), free);
    }

    #[test]
    fn a_busy_preferred_port_falls_back_to_a_free_one() {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let busy = listener.local_addr().unwrap().port();
        let chosen = choose_port(busy);
        assert_ne!(chosen, busy);
        assert!(port_is_free(chosen), "the fallback port is free");
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
        assert!(!answering(config.port));
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

    /// A child that dies while Hex owns it is started again after the pause,
    /// not left as a dead server.
    #[cfg(windows)]
    #[test]
    fn an_unexpected_exit_schedules_a_restart() {
        let dir = temp_dir("restart");
        let mut model = LocalModel::new(Config {
            program: dir.join("absent").join(program_name()),
            model: dir.join("model.gguf"),
            port: 9,
            args: Vec::new(),
        });
        model.state = State::Ready;
        let mut child = std::process::Command::new("cmd")
            .args(["/c", "exit", "1"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("a child that exits");
        let _ = child.wait();
        model.child = Some(child);
        model.poll();
        assert_eq!(model.state, State::Starting);
        assert_eq!(model.restarts, 1);
        assert!(model.restart_at.is_some());
        assert!(
            model
                .detail
                .as_deref()
                .is_some_and(|detail| detail.contains("exit code: 1")),
            "{:?}",
            model.detail
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A planned stop clears the child and never looks like an unexpected exit.
    #[test]
    fn a_planned_stop_does_not_schedule_a_restart() {
        let mut model = LocalModel::new(Config::default());
        model.state = State::Ready;
        model.stop();
        assert_eq!(model.state, State::Stopped);
        assert!(model.restart_at.is_none());
        assert!(model.ready_at.is_none());
    }

    /// A server that cannot stay up is not restarted forever.
    #[test]
    fn a_server_that_keeps_exiting_stops_after_three_restarts() {
        let mut model = LocalModel::new(Config::default());
        model.state = State::Ready;
        model.restart_window = Some(Instant::now());
        model.restarts = RESTART_LIMIT;
        model.restart_after_exit("llama-server exited (exit code: 1)".into());
        assert_eq!(model.state, State::Failed);
        assert!(model.restart_at.is_none());
        assert!(
            model
                .detail
                .as_deref()
                .is_some_and(|detail| detail.contains("logs/llama-server.log")),
            "{:?}",
            model.detail
        );
    }

    /// The job object is what makes the child die with Hex; closing the handle
    /// is the same event a crash or a force kill produces.
    #[cfg(windows)]
    #[test]
    fn closing_the_job_ends_the_server_child() {
        use crate::personal_commands::process::windows_job::Job;
        let mut child = std::process::Command::new("cmd")
            .args(["/c", "ping", "-n", "30", "127.0.0.1"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("a child to tie to the job");
        let job = Job::new().expect("a job object");
        job.assign(&child).expect("the child joins the job");
        drop(job);
        let ended = wait_for_exit(&mut child);
        let _ = child.kill();
        assert!(ended, "closing the job did not end the child");
    }

    /// A pid Hex recorded at spawn and never cleared is killed at the next
    /// start, before the next start chooses a port. Hex never kills a pid that
    /// is its own, and clears the record either way.
    #[cfg(windows)]
    #[test]
    fn a_recorded_pid_from_a_previous_run_is_stopped() {
        let dir = temp_dir("orphan");
        let path = dir.join("local-model.pid");
        std::fs::write(&path, std::process::id().to_string()).unwrap();
        let mut model = LocalModel::new(Config::default());
        model.kill_orphan_at(&path);
        assert!(process_alive(std::process::id()), "Hex never kills itself");
        assert!(!path.exists(), "the recorded pid is cleared");

        let mut child = fake_server(&dir);
        std::fs::write(&path, child.id().to_string()).unwrap();
        model.kill_orphan_at(&path);
        let ended = wait_for_exit(&mut child);
        let _ = child.kill();
        assert!(ended, "the recorded orphan kept running");
        assert!(!path.exists(), "the recorded pid is cleared");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A running process whose file name is `llama-server.exe`, so the orphan
    /// cleanup's own-name check admits it.
    #[cfg(windows)]
    fn fake_server(dir: &std::path::Path) -> Child {
        let program = dir.join("llama-server.exe");
        if !program.exists() {
            std::fs::copy(r"C:\Windows\System32\cmd.exe", &program).unwrap();
        }
        std::process::Command::new(&program)
            .args(["/c", "ping", "-n", "30", "127.0.0.1"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("a fake server to stop")
    }

    #[cfg(windows)]
    fn wait_for_exit(child: &mut Child) -> bool {
        for _ in 0..100 {
            if child.try_wait().ok().flatten().is_some() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    }
}
