//! One script worker thread: a QuickJS runtime and context, the prelude,
//! and the modules evaluated in it (by source hash), plus each script's
//! `context.shared`.
//!
//! The context is rebuilt (dropping every module and `shared` object)
//! after an interrupt or an out-of-memory error, and once 64 modules have
//! been declared, since QuickJS never unloads a module.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::mpsc::Receiver;
use std::time::Instant;

use pm_model::{BodyPart, ScriptId, ScriptPhase, ScriptViewKind};
use rquickjs::promise::PromiseState;
use rquickjs::{
    Array, CatchResultExt, CaughtError, Context, Ctx, Function, Module, Object, Persistent, Promise, Runtime,
    TypedArray, Value,
};

use super::host::{
    Call, CallInfo, Code, Exports, Job, JsRequest, JsResponse, MockOut, Output, RequestOut, ResponseOut, RunResult,
    ViewOut,
};
use super::native::{self, ConsoleBuffer, bytes_of};

/// QuickJS heap per worker.
pub(crate) const MEMORY_LIMIT: usize = 256 * 1024 * 1024;
/// QuickJS stack per worker (the thread's own stack is larger).
const STACK_LIMIT: usize = 1024 * 1024;
/// Modules declared before the context is rebuilt.
const MODULES_MAX: usize = 64;

const PRELUDE: &str = include_str!("prelude.js");

/// The worker loop: runs jobs until the host is dropped.
pub(super) fn run(jobs: Receiver<Job>) {
    let mut worker: Option<Worker> = None;
    while let Ok(job) = jobs.recv() {
        if Instant::now() >= job.deadline {
            job.reply
                .send(RunResult::failed("timed out waiting for the script worker".into()));
            continue;
        }
        let current = match &mut worker {
            Some(w) => w,
            None => match Worker::new() {
                Ok(w) => worker.insert(w),
                Err(err) => {
                    job.reply.send(RunResult::failed(format!("can't start QuickJS: {err}")));
                    continue;
                }
            },
        };
        let Job {
            code,
            call,
            info,
            deadline,
            ephemeral,
            reply,
        } = job;
        let (result, rebuild) = current.execute(&code, call, &info, deadline, ephemeral);
        if rebuild {
            worker = None;
        }
        reply.send(result);
    }
}

/// Handles kept across runs. Dropped before the context they point into.
#[derive(Default)]
struct Handles {
    invoke: Option<Persistent<Function<'static>>>,
    /// Module namespaces by source hash.
    modules: HashMap<u64, Persistent<Object<'static>>>,
    /// Each script's `context.shared`, for the source it was made for.
    shared: HashMap<ScriptId, (u64, Persistent<Object<'static>>)>,
    declared: usize,
}

struct Worker {
    // Field order is drop order: the handles go before the context and
    // the runtime they belong to.
    handles: Handles,
    deadline: Rc<Cell<Option<Instant>>>,
    console: ConsoleBuffer,
    context: Context,
    _runtime: Runtime,
}

impl Worker {
    fn new() -> rquickjs::Result<Worker> {
        let runtime = Runtime::new()?;
        runtime.set_memory_limit(MEMORY_LIMIT);
        runtime.set_max_stack_size(STACK_LIMIT);
        let deadline: Rc<Cell<Option<Instant>>> = Rc::new(Cell::new(None));
        let watched = deadline.clone();
        runtime.set_interrupt_handler(Some(Box::new(move || {
            watched.get().is_some_and(|d| Instant::now() >= d)
        })));
        let context = Context::full(&runtime)?;
        let console: ConsoleBuffer = Rc::new(RefCell::new(Vec::new()));
        let mut handles = Handles::default();
        context.with(|ctx| -> rquickjs::Result<()> {
            let native = Object::new(ctx.clone())?;
            native::install(&ctx, &native, console.clone())?;
            let setup: Function = ctx.eval(PRELUDE)?;
            let internals: Object = setup.call((native,))?;
            let invoke: Function = internals.get("invoke")?;
            handles.invoke = Some(Persistent::save(&ctx, invoke));
            Ok(())
        })?;
        Ok(Worker {
            handles,
            deadline,
            console,
            context,
            _runtime: runtime,
        })
    }

    /// Runs one job. The flag asks for the context to be rebuilt.
    fn execute(
        &mut self,
        code: &Code,
        call: Call,
        info: &CallInfo,
        deadline: Instant,
        ephemeral: bool,
    ) -> (RunResult, bool) {
        self.console.borrow_mut().clear();
        self.deadline.set(Some(deadline));
        let started = Instant::now();
        let handles = &mut self.handles;
        let output = self
            .context
            .with(|ctx| execute(&ctx, handles, code, call, info, deadline, ephemeral));
        self.deadline.set(None);
        let timed_out = Instant::now() >= deadline;
        let mut rebuild = self.handles.declared >= MODULES_MAX;
        let output = output.map_err(|message| {
            if timed_out {
                rebuild = true;
                let ms = deadline.saturating_duration_since(started).as_millis();
                format!("timed out after {ms} ms")
            } else if message.contains("out of memory") {
                rebuild = true;
                format!(
                    "out of memory (scripts get {} MB); the script engine was restarted",
                    MEMORY_LIMIT / 1024 / 1024
                )
            } else if message.contains("stack overflow") {
                rebuild = true;
                message
            } else {
                message
            }
        });
        let console = std::mem::take(&mut *self.console.borrow_mut());
        (
            RunResult {
                output,
                console,
                duration: started.elapsed(),
            },
            rebuild,
        )
    }
}

fn caught(err: CaughtError<'_>) -> String {
    match err {
        CaughtError::Exception(e) => {
            let message = e.message().unwrap_or_default();
            let stack = e.stack().unwrap_or_default();
            let name: String = e.get("name").unwrap_or_else(|_| "Error".into());
            let head = if message.is_empty() {
                name
            } else {
                format!("{name}: {message}")
            };
            // The prelude's own frames say nothing about the script.
            let stack: Vec<&str> = stack
                .lines()
                .filter(|l| !l.trim().is_empty() && !l.contains("(eval_script:"))
                .collect();
            if stack.is_empty() {
                head
            } else {
                format!("{head}\n{}", stack.join("\n"))
            }
        }
        CaughtError::Value(v) => format!("threw {}", describe(&v)),
        CaughtError::Error(e) => e.to_string(),
    }
}

fn describe(value: &Value<'_>) -> String {
    if let Some(s) = value.as_string().and_then(|s| s.to_string().ok()) {
        return s;
    }
    value
        .ctx()
        .json_stringify(value.clone())
        .ok()
        .flatten()
        .and_then(|s| s.to_string().ok())
        .unwrap_or_else(|| format!("a {}", value.type_name()))
}

/// Runs the job queue until `promise` settles.
fn settle<'js>(ctx: &Ctx<'js>, promise: &Promise<'js>, deadline: Instant) -> Result<Value<'js>, String> {
    while promise.state() == PromiseState::Pending {
        if !ctx.execute_pending_job() {
            break;
        }
    }
    match promise.result::<Value>() {
        Some(result) => result.catch(ctx).map_err(caught),
        None if Instant::now() >= deadline => Err("timed out".into()),
        None => Err(
            "the promise never settled: scripts have no timers or network, so only promises they resolve \
             themselves can finish"
                .into(),
        ),
    }
}

/// The module for `code`: cached by source hash, or evaluated now.
fn module<'js>(
    ctx: &Ctx<'js>,
    handles: &mut Handles,
    code: &Code,
    ephemeral: bool,
    deadline: Instant,
) -> Result<Object<'js>, String> {
    if !ephemeral && let Some(ns) = handles.modules.get(&code.hash) {
        return ns.clone().restore(ctx).map_err(|e| e.to_string());
    }
    handles.declared += 1;
    let name = if code.name.trim().is_empty() {
        "script".to_owned()
    } else {
        code.name.clone()
    };
    let declared = Module::declare(ctx.clone(), name, code.source.as_bytes())
        .catch(ctx)
        .map_err(caught)?;
    let (evaluated, promise) = declared.eval().catch(ctx).map_err(caught)?;
    settle(ctx, &promise, deadline)?;
    let ns = evaluated.namespace().catch(ctx).map_err(caught)?;
    if !ephemeral {
        handles.modules.insert(code.hash, Persistent::save(ctx, ns.clone()));
    }
    Ok(ns)
}

fn shared<'js>(ctx: &Ctx<'js>, handles: &mut Handles, code: &Code, ephemeral: bool) -> Result<Object<'js>, String> {
    if !ephemeral
        && let Some((hash, obj)) = handles.shared.get(&code.id)
        && *hash == code.hash
    {
        return obj.clone().restore(ctx).map_err(|e| e.to_string());
    }
    let obj = Object::new(ctx.clone()).map_err(|e| e.to_string())?;
    if !ephemeral {
        handles
            .shared
            .insert(code.id.clone(), (code.hash, Persistent::save(ctx, obj.clone())));
    }
    Ok(obj)
}

fn exports(ns: &Object<'_>) -> Exports {
    let is_fn = |name: &str| ns.get::<_, Value>(name).is_ok_and(|v| v.is_function());
    Exports {
        on_request: is_fn("onRequest"),
        on_response: is_fn("onResponse"),
        on_mock: is_fn("onMock"),
        on_view: is_fn("onView"),
    }
}

fn execute<'js>(
    ctx: &Ctx<'js>,
    handles: &mut Handles,
    code: &Code,
    call: Call,
    info: &CallInfo,
    deadline: Instant,
    ephemeral: bool,
) -> Result<Output, String> {
    let ns = module(ctx, handles, code, ephemeral, deadline)?;
    let (export, phase) = match &call {
        Call::Inspect => return Ok(Output::Exports(exports(&ns))),
        Call::Request(_) => ("onRequest", ScriptPhase::Request),
        Call::Response(..) => ("onResponse", ScriptPhase::Response),
        Call::Mock(_) => ("onMock", ScriptPhase::Mock),
        Call::View(..) => ("onView", ScriptPhase::View),
    };
    let entry: Value = ns.get(export).map_err(|e| e.to_string())?;
    if !entry.is_function() {
        return Ok(Output::Missing);
    }
    let shared = shared(ctx, handles, code, ephemeral)?;
    let invoke = handles
        .invoke
        .clone()
        .ok_or("the script engine isn't set up")?
        .restore(ctx)
        .map_err(|e| e.to_string())?;
    let context = context_info(ctx, code, phase, info).map_err(|e| e.to_string())?;
    let js = |r: rquickjs::Result<Value<'js>>| r.map_err(|e| e.to_string());
    let (a, b) = match &call {
        Call::Inspect => unreachable!(),
        Call::Request(req) | Call::Mock(req) => (js(request_js(ctx, req))?, Value::new_undefined(ctx.clone())),
        Call::Response(req, resp) => (js(request_js(ctx, req))?, js(response_js(ctx, resp))?),
        Call::View(req, resp) => (
            js(request_js(ctx, req))?,
            match resp {
                Some(resp) => js(response_js(ctx, resp))?,
                None => Value::new_null(ctx.clone()),
            },
        ),
    };
    let phase_name = match phase {
        ScriptPhase::Request => "request",
        ScriptPhase::Response => "response",
        ScriptPhase::Mock => "mock",
        ScriptPhase::View => "view",
    };
    let promise: Promise = invoke
        .call((entry, phase_name, context, shared, a, b))
        .catch(ctx)
        .map_err(caught)?;
    let result = settle(ctx, &promise, deadline)?;
    let output = match call {
        Call::Inspect => unreachable!(),
        Call::Request(_) => Output::Request(request_out(&result)?),
        Call::Response(..) => Output::Response(response_out(&result)?),
        Call::Mock(_) => Output::Mock(mock_out(&result)?),
        Call::View(..) => Output::Views(views_out(&result)?),
    };
    Ok(output)
}

fn context_info<'js>(
    ctx: &Ctx<'js>,
    code: &Code,
    phase: ScriptPhase,
    info: &CallInfo,
) -> rquickjs::Result<Object<'js>> {
    let obj = Object::new(ctx.clone())?;
    obj.set("scriptId", code.id.as_str())?;
    obj.set("scriptName", code.name.as_str())?;
    obj.set("flowId", info.flow_id)?;
    obj.set(
        "phase",
        match phase {
            ScriptPhase::Request => "request",
            ScriptPhase::Response => "response",
            ScriptPhase::Mock => "mock",
            ScriptPhase::View => "view",
        },
    )?;
    match &info.rule {
        Some((id, name)) => {
            let rule = Object::new(ctx.clone())?;
            rule.set("id", id.as_str())?;
            rule.set("name", name.as_str())?;
            obj.set("rule", rule)?;
        }
        None => obj.set("rule", Value::new_null(ctx.clone()))?,
    }
    obj.set(
        "part",
        info.part.map(|p| match p {
            BodyPart::Request => "request",
            BodyPart::Response => "response",
        }),
    )?;
    Ok(obj)
}

fn headers_js<'js>(ctx: &Ctx<'js>, headers: &[(String, String)]) -> rquickjs::Result<Array<'js>> {
    let list = Array::new(ctx.clone())?;
    for (i, (name, value)) in headers.iter().enumerate() {
        let pair = Array::new(ctx.clone())?;
        pair.set(0, name.as_str())?;
        pair.set(1, value.as_str())?;
        list.set(i, pair)?;
    }
    Ok(list)
}

fn body_js<'js>(ctx: &Ctx<'js>, obj: &Object<'js>, body: Option<&Vec<u8>>) -> rquickjs::Result<()> {
    match body {
        Some(bytes) => obj.set("body", TypedArray::<u8>::new_copy(ctx.clone(), bytes.as_slice())?),
        None => obj.set("bodyTooLarge", true),
    }
}

fn request_js<'js>(ctx: &Ctx<'js>, req: &JsRequest) -> rquickjs::Result<Value<'js>> {
    let obj = Object::new(ctx.clone())?;
    obj.set("method", req.method.as_str())?;
    obj.set("url", req.url.as_str())?;
    obj.set("scheme", req.scheme.as_str())?;
    obj.set("host", req.host.as_str())?;
    obj.set("port", req.port)?;
    obj.set("path", req.path.as_str())?;
    obj.set("query", req.query.as_str())?;
    obj.set("headers", headers_js(ctx, &req.headers)?)?;
    body_js(ctx, &obj, req.body.as_deref())?;
    Ok(obj.into_value())
}

fn response_js<'js>(ctx: &Ctx<'js>, resp: &JsResponse) -> rquickjs::Result<Value<'js>> {
    let obj = Object::new(ctx.clone())?;
    obj.set("status", resp.status)?;
    obj.set("headers", headers_js(ctx, &resp.headers)?)?;
    body_js(ctx, &obj, resp.body.as_deref())?;
    Ok(obj.into_value())
}

fn get<'js, T: rquickjs::FromJs<'js>>(obj: &Object<'js>, key: &str) -> Result<T, String> {
    obj.get(key).map_err(|e| format!("{key}: {e}"))
}

fn object<'js>(value: &Value<'js>, what: &str) -> Result<Object<'js>, String> {
    value
        .as_object()
        .cloned()
        .ok_or_else(|| format!("{what} must be an object"))
}

fn optional_headers(obj: &Object<'_>) -> Result<Option<Vec<(String, String)>>, String> {
    let value: Value = get(obj, "headers")?;
    if value.is_null() || value.is_undefined() {
        return Ok(None);
    }
    header_list(&value).map(Some)
}

fn header_list(value: &Value<'_>) -> Result<Vec<(String, String)>, String> {
    let list = value.as_array().ok_or("headers must be a list")?;
    let mut out = Vec::with_capacity(list.len());
    for pair in list.iter::<Array>() {
        let pair = pair.map_err(|e| format!("headers: {e}"))?;
        let name: String = pair.get(0).map_err(|e| format!("headers: {e}"))?;
        let value: String = pair.get(1).map_err(|e| format!("headers: {e}"))?;
        out.push((name, value));
    }
    Ok(out)
}

fn optional_bytes(obj: &Object<'_>, key: &str) -> Result<Option<Vec<u8>>, String> {
    let value: Value = get(obj, key)?;
    if value.is_null() || value.is_undefined() {
        return Ok(None);
    }
    bytes_of(&value).map(Some).ok_or_else(|| format!("{key} must be bytes"))
}

fn request_out(result: &Value<'_>) -> Result<RequestOut, String> {
    let obj = object(result, "the request")?;
    let port: f64 = get(&obj, "port")?;
    if !(1.0..=65535.0).contains(&port) || port.fract() != 0.0 {
        return Err(format!("{port} isn't a port"));
    }
    Ok(RequestOut {
        method: get(&obj, "method")?,
        scheme: get(&obj, "scheme")?,
        host: get(&obj, "host")?,
        port: port as u16,
        path: get(&obj, "path")?,
        query: get(&obj, "query")?,
        headers: optional_headers(&obj)?,
        body: optional_bytes(&obj, "body")?,
    })
}

fn status(obj: &Object<'_>) -> Result<u16, String> {
    let status: f64 = get(obj, "status")?;
    if !(100.0..=999.0).contains(&status) || status.fract() != 0.0 {
        return Err(format!("{status} isn't a status code"));
    }
    Ok(status as u16)
}

fn response_out(result: &Value<'_>) -> Result<ResponseOut, String> {
    let obj = object(result, "the response")?;
    Ok(ResponseOut {
        status: status(&obj)?,
        headers: optional_headers(&obj)?,
        body: optional_bytes(&obj, "body")?,
    })
}

fn mock_out(result: &Value<'_>) -> Result<Option<MockOut>, String> {
    if result.is_null() || result.is_undefined() {
        return Ok(None);
    }
    let obj = object(result, "the mock")?;
    Ok(Some(MockOut {
        status: status(&obj)?,
        headers: header_list(&get::<Value>(&obj, "headers")?)?,
        body: optional_bytes(&obj, "body")?.unwrap_or_default(),
    }))
}

fn views_out(result: &Value<'_>) -> Result<Vec<ViewOut>, String> {
    let list = result.as_array().ok_or("onView must return views")?;
    let mut views = Vec::with_capacity(list.len());
    for item in list.iter::<Object>() {
        let item = item.map_err(|e| e.to_string())?;
        let kind: String = get(&item, "kind")?;
        let kind = match kind.as_str() {
            "json" => ScriptViewKind::Json,
            "xml" => ScriptViewKind::Xml,
            "html" => ScriptViewKind::Html,
            "hex" => ScriptViewKind::Hex,
            "image" => ScriptViewKind::Image,
            _ => ScriptViewKind::Text,
        };
        views.push(ViewOut {
            title: get(&item, "title")?,
            kind,
            text: get(&item, "text")?,
            bytes: optional_bytes(&item, "bytes")?,
            mime: get(&item, "mime")?,
        });
    }
    Ok(views)
}
