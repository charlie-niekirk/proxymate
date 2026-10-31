//! [`ScriptHost`]: N dedicated threads, each owning one QuickJS runtime and
//! context (a `Runtime` isn't `Send` without rquickjs's `parallel`
//! feature). Jobs are sharded by script id, so a script always runs on the
//! same worker and its `context.shared` stays consistent. Each worker has
//! a bounded queue; a full queue is a script error, never back-pressure on
//! the proxy. Answers come back over a tokio oneshot.
//!
//! Every job carries an absolute deadline, set when it is queued: a job
//! still waiting when it passes fails without running, and a running one
//! is interrupted. So a flow never waits on a script for longer than the
//! timeout, however busy the worker.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::sync::mpsc::{SyncSender, TrySendError};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use pm_model::{BodyPart, ConsoleLevel, FlowId, ScriptId, ScriptPhase, ScriptViewKind};

use super::worker;

/// Jobs waiting per worker before new ones are refused.
const QUEUE: usize = 256;
/// Each worker thread's stack. QuickJS's own limit is far lower.
const THREAD_STACK: usize = 8 * 1024 * 1024;

/// What a script exports, found when it is saved.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Exports {
    pub on_request: bool,
    pub on_response: bool,
    pub on_mock: bool,
    pub on_view: bool,
    pub on_ws_message: bool,
}

/// The code a job runs.
#[derive(Clone)]
pub(crate) struct Code {
    pub id: ScriptId,
    pub name: String,
    pub source: Arc<str>,
    pub hash: u64,
}

impl Code {
    pub fn new(id: ScriptId, name: String, source: Arc<str>) -> Code {
        let mut hasher = DefaultHasher::new();
        source.hash(&mut hasher);
        Code {
            id,
            name,
            source,
            hash: hasher.finish(),
        }
    }
}

/// A request as a script sees it.
#[derive(Debug, Clone)]
pub(crate) struct JsRequest {
    pub method: String,
    pub url: String,
    pub scheme: String,
    pub host: String,
    pub port: u16,
    pub path: String,
    /// Raw, without the `?`.
    pub query: String,
    pub headers: Vec<(String, String)>,
    /// Decoded. `None` when it wasn't buffered or is over the script cap.
    pub body: Option<Arc<Vec<u8>>>,
}

#[derive(Debug, Clone)]
pub(crate) struct JsResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Option<Arc<Vec<u8>>>,
}

/// A WebSocket message as a script sees it.
#[derive(Debug, Clone)]
pub(crate) struct JsWsMessage {
    pub direction: pm_model::WsDirection,
    pub text: bool,
    pub payload: Arc<Vec<u8>>,
}

/// What `onWsMessage` did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WsScriptOut {
    /// Returned nothing or the message untouched.
    Keep,
    /// Returned `null`.
    Drop,
    /// A changed or replacement message: its type, and its payload
    /// (`None`: the payload is as it came).
    Replace { text: bool, payload: Option<Vec<u8>> },
}

/// What a request script left. `None` fields are unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RequestOut {
    pub method: String,
    pub scheme: String,
    pub host: String,
    pub port: u16,
    pub path: String,
    pub query: String,
    pub headers: Option<Vec<(String, String)>>,
    pub body: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResponseOut {
    pub status: u16,
    pub headers: Option<Vec<(String, String)>>,
    pub body: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MockOut {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ViewOut {
    pub title: String,
    pub kind: ScriptViewKind,
    pub text: Option<String>,
    pub bytes: Option<Vec<u8>>,
    pub mime: Option<String>,
}

/// The entry point to run, with its arguments.
pub(crate) enum Call {
    /// Evaluates the module and reports its exports.
    Inspect,
    Request(JsRequest),
    Response(JsRequest, JsResponse),
    Mock(JsRequest),
    View(JsRequest, Option<JsResponse>),
    WsMessage(JsWsMessage),
}

impl Call {
    pub fn phase(&self) -> Option<ScriptPhase> {
        match self {
            Call::Inspect => None,
            Call::Request(_) => Some(ScriptPhase::Request),
            Call::Response(..) => Some(ScriptPhase::Response),
            Call::Mock(_) => Some(ScriptPhase::Mock),
            Call::View(..) => Some(ScriptPhase::View),
            Call::WsMessage(_) => Some(ScriptPhase::WsMessage),
        }
    }
}

#[derive(Debug)]
pub(crate) enum Output {
    Exports(Exports),
    /// The export the call needs is missing: nothing ran.
    Missing,
    Request(RequestOut),
    Response(ResponseOut),
    Mock(Option<MockOut>),
    Views(Vec<ViewOut>),
    WsMessage(WsScriptOut),
}

/// `context` minus `shared`.
#[derive(Debug, Clone, Default)]
pub(crate) struct CallInfo {
    pub flow_id: Option<FlowId>,
    pub rule: Option<(String, String)>,
    pub part: Option<BodyPart>,
}

/// A run's result and what the script logged.
#[derive(Debug)]
pub(crate) struct RunResult {
    pub output: Result<Output, String>,
    pub console: Vec<(ConsoleLevel, String)>,
    pub duration: Duration,
}

impl RunResult {
    pub fn failed(message: String) -> RunResult {
        RunResult {
            output: Err(message),
            console: Vec::new(),
            duration: Duration::ZERO,
        }
    }
}

pub(crate) struct Job {
    pub code: Code,
    pub call: Call,
    pub info: CallInfo,
    pub deadline: Instant,
    /// Test runs evaluate a fresh module and don't keep it.
    pub ephemeral: bool,
    pub reply: Reply,
}

pub(crate) enum Reply {
    Async(tokio::sync::oneshot::Sender<RunResult>),
    Blocking(std::sync::mpsc::Sender<RunResult>),
}

impl Reply {
    pub fn send(self, result: RunResult) {
        match self {
            Reply::Async(tx) => {
                let _ = tx.send(result);
            }
            Reply::Blocking(tx) => {
                let _ = tx.send(result);
            }
        }
    }
}

/// The worker threads, started on first use.
pub(crate) struct ScriptHost {
    workers: Mutex<Vec<SyncSender<Job>>>,
    count: usize,
}

impl ScriptHost {
    pub fn new() -> ScriptHost {
        let cores = std::thread::available_parallelism().map_or(2, |n| n.get());
        ScriptHost {
            workers: Mutex::new(Vec::new()),
            count: (cores / 2).clamp(1, 4),
        }
    }

    /// Runs `call` on the script's worker, waiting at most `timeout`.
    pub async fn run(&self, code: Code, call: Call, info: CallInfo, timeout: Duration, ephemeral: bool) -> RunResult {
        let (reply, answer) = tokio::sync::oneshot::channel();
        let job = Job {
            code,
            call,
            info,
            deadline: Instant::now() + timeout,
            ephemeral,
            reply: Reply::Async(reply),
        };
        if let Err(message) = self.submit(job) {
            return RunResult::failed(message);
        }
        answer
            .await
            .unwrap_or_else(|_| RunResult::failed("the script worker stopped".into()))
    }

    /// Like [`ScriptHost::run`], blocking the calling thread. For saving a
    /// script, which may run inside or outside a tokio runtime.
    pub fn run_blocking(&self, code: Code, call: Call, timeout: Duration) -> RunResult {
        let (reply, answer) = std::sync::mpsc::channel();
        let job = Job {
            code,
            call,
            info: CallInfo::default(),
            deadline: Instant::now() + timeout,
            ephemeral: false,
            reply: Reply::Blocking(reply),
        };
        if let Err(message) = self.submit(job) {
            return RunResult::failed(message);
        }
        answer
            .recv()
            .unwrap_or_else(|_| RunResult::failed("the script worker stopped".into()))
    }

    fn submit(&self, job: Job) -> Result<(), String> {
        let mut workers = self.workers.lock();
        if workers.is_empty() {
            // The last worker runs only test runs, so their throwaway
            // modules never force a traffic worker to rebuild.
            for i in 0..=self.count {
                let (tx, rx) = std::sync::mpsc::sync_channel(QUEUE);
                std::thread::Builder::new()
                    .name(format!("pm-script-{i}"))
                    .stack_size(THREAD_STACK)
                    .spawn(move || worker::run(rx))
                    .map_err(|e| format!("can't start a script worker: {e}"))?;
                workers.push(tx);
            }
        }
        let index = if job.ephemeral {
            self.count
        } else {
            let mut hasher = DefaultHasher::new();
            job.code.id.hash(&mut hasher);
            (hasher.finish() % self.count as u64) as usize
        };
        match workers[index].try_send(job) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => Err("the script worker is busy (256 runs waiting); skipped".into()),
            Err(TrySendError::Disconnected(_)) => Err("the script worker stopped".into()),
        }
    }
}
