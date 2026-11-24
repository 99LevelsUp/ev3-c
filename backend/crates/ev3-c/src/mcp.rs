//! `ev3 c mcp`: EVƎ C's project operations as MCP tools for an agent, over stdio.
//!
//! - Standard output carries MCP messages only, one JSON-RPC message per line. The server ends
//!   when its client closes standard input.
//! - The tools are the `ev3 c` commands that work on a project: build, upload, run, and the
//!   project's runs. Bricks, files, stopping and telemetry are EVƎ Cockpit's own MCP tools
//!   (`ev3 mcp`); nothing here repeats them.
//! - Each tool runs the command with --json and answers with its object. Nothing reaches the
//!   cockpit's daemon until a tool needs a brick, and then as kind `mcp`, named after the
//!   agent, which can never confirm anything on a person's behalf.
//! - A project folder must lie inside the roots the MCP client declares, or inside the working
//!   folder when it declares none: the tools build there and write returned files below it.
//! - Cancelling a run stops the program, as Ctrl+C does in a terminal.
//!
//! Requests are handled side by side on one thread: a tool call asks the client for its roots
//! and waits for the answer on the same standard input, and a cancel must reach a run that is
//! still going.

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use serde_json::{Map, Value, json};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::{mpsc, oneshot};

use ev3c_core::port::Clock;

use crate::cli::{Client, Deps, Interrupts, Io, run_cli};
use crate::cockpit::Connector;
use crate::paths::{file_uri_to_path, real_path, resolve};

/// The protocol versions this server speaks; the first is the newest.
pub const PROTOCOL_VERSIONS: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

/// How this server introduces itself to the daemon when the client gave no name.
pub const MCP_CLIENT_NAME: &str = "agent";

/// How long the server waits for the client to list its roots.
const ROOTS_WAIT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy)]
enum Kind {
    Text,
    WholeNumber { minimum: i64 },
}

#[derive(Debug, Clone, Copy)]
struct Property {
    name: &'static str,
    kind: Kind,
    description: &'static str,
}

/// One tool: what `tools/list` shows, and the `ev3 c` arguments for a call, whose project
/// folder is resolved and checked already.
#[derive(Debug, Clone, Copy)]
pub struct Tool {
    pub name: &'static str,
    pub description: &'static str,
    properties: &'static [Property],
    required: &'static [&'static str],
    argv: fn(&Map<String, Value>, &str) -> Vec<String>,
}

impl Tool {
    pub fn input_schema(&self) -> Value {
        let properties: Map<String, Value> = self
            .properties
            .iter()
            .map(|property| {
                let schema = match property.kind {
                    Kind::Text => json!({ "type": "string", "description": property.description }),
                    Kind::WholeNumber { minimum } => json!({
                        "type": "integer",
                        "minimum": minimum,
                        "description": property.description,
                    }),
                };
                (property.name.to_owned(), schema)
            })
            .collect();
        json!({
            "type": "object",
            "properties": properties,
            "required": self.required,
            "additionalProperties": false,
        })
    }
}

const PROJECT: Property = Property {
    name: "project",
    kind: Kind::Text,
    description: "The project folder on this computer: the folder that holds ev3c.json.",
};

const BRICK: Property = Property {
    name: "brick",
    kind: Kind::Text,
    description: "The brick, by name or id. Default: the brick EVƎ Cockpit's VS Code window shows, else the only connected brick.",
};

pub const TOOLS: [Tool; 6] = [
    Tool {
        name: "build_project",
        description: "EVƎ C: Build an EVƎ C project (C for the LEGO EV3 with its original firmware) into the program, its launcher and the supervisor. Returns the artifacts and any compiler warnings; a failed build returns the compiler errors.",
        properties: &[PROJECT],
        required: &["project"],
        argv: |_, project| vec!["build".to_owned(), project.to_owned()],
    },
    Tool {
        name: "upload_project",
        description: "EVƎ C: Build an EVƎ C project and upload what changed to the brick, without running it. Refused while a program runs on the brick.",
        properties: &[PROJECT, BRICK],
        required: &["project"],
        argv: |args, project| with_brick(args, vec!["upload".to_owned(), project.to_owned()]),
    },
    Tool {
        name: "run_project",
        description: "EVƎ C: Build an EVƎ C project, upload it and run it on the brick, then wait for the run to end and return its result: how it ended (exit code or signal), its stdout and stderr, and the files it returned, which are downloaded into the project's returned/run-<n> folder. Cancelling the call stops the program. To stop a program you did not start here, use EVƎ Cockpit's stop_program.",
        properties: &[PROJECT, BRICK],
        required: &["project"],
        argv: |args, project| with_brick(args, vec!["run".to_owned(), project.to_owned()]),
    },
    Tool {
        name: "list_project_runs",
        description: "EVƎ C: List an EVƎ C project's runs kept on the brick, oldest first, with how each ended; a run without a record is still running or was interrupted.",
        properties: &[PROJECT, BRICK],
        required: &["project"],
        argv: |args, project| with_brick(args, vec!["runs".to_owned(), project.to_owned()]),
    },
    Tool {
        name: "show_project_run",
        description: "EVƎ C: Return one run of an EVƎ C project: how it ended, its output, and the files it returned, downloaded into the project's returned/run-<n> folder.",
        properties: &[
            PROJECT,
            Property {
                name: "run",
                kind: Kind::WholeNumber { minimum: 1 },
                description: "The run number, as list_project_runs gives it.",
            },
            BRICK,
        ],
        required: &["project", "run"],
        argv: |args, project| {
            let run = args.get("run").map(number_text).unwrap_or_default();
            with_brick(
                args,
                vec![
                    "runs".to_owned(),
                    "show".to_owned(),
                    run,
                    project.to_owned(),
                ],
            )
        },
    },
    Tool {
        name: "delete_project_runs",
        description: "EVƎ C: Delete an EVƎ C project's run folders on the brick, all of them or all but the newest `keep`. The folder of a run in progress is never deleted.",
        properties: &[
            PROJECT,
            Property {
                name: "keep",
                kind: Kind::WholeNumber { minimum: 0 },
                description: "How many of the newest runs to keep; 0 or none deletes all.",
            },
            BRICK,
        ],
        required: &["project"],
        argv: |args, project| {
            let mut argv = vec!["runs".to_owned(), "delete".to_owned(), project.to_owned()];
            if let Some(keep) = args.get("keep") {
                argv.extend(["--keep".to_owned(), number_text(keep)]);
            }
            with_brick(args, argv)
        },
    },
];

fn with_brick(args: &Map<String, Value>, mut argv: Vec<String>) -> Vec<String> {
    if let Some(brick) = args
        .get("brick")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|brick| !brick.is_empty())
    {
        argv.extend(["--brick".to_owned(), brick.to_owned()]);
    }
    argv
}

/// A whole number as JavaScript's `String()` writes it; `2.0` is `2`.
fn number_text(value: &Value) -> String {
    match (value.as_i64(), value.as_u64(), value.as_f64()) {
        (Some(number), _, _) => number.to_string(),
        (None, Some(number), _) => number.to_string(),
        (None, None, Some(number)) => format!("{number:.0}"),
        _ => value.to_string(),
    }
}

/// A number with no fraction, as `Number.isInteger` sees it: JSON's `2.0` is one.
fn whole(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .filter(|number| number.is_finite() && number.fract() == 0.0)
}

/// What is wrong with a call's arguments, if anything.
fn validate(tool: &Tool, args: &Map<String, Value>) -> Option<String> {
    if let Some(missing) = tool.required.iter().find(|name| !args.contains_key(**name)) {
        return Some(format!("\"{missing}\" is required"));
    }
    for (name, value) in args {
        let Some(property) = tool.properties.iter().find(|p| p.name == name) else {
            return Some(format!("unknown argument \"{name}\""));
        };
        match property.kind {
            Kind::Text if !value.is_string() => {
                return Some(format!("\"{name}\" must be a string"));
            }
            Kind::WholeNumber { minimum }
                if !whole(value).is_some_and(|number| number >= minimum as f64) =>
            {
                return Some(format!("\"{name}\" must be a whole number from {minimum}"));
            }
            _ => {}
        }
    }
    None
}

fn tool_error(text: impl Into<String>) -> Value {
    json!({ "content": [{ "type": "text", "text": text.into() }], "isError": true })
}

/// Keys requests by id as JavaScript's `String(id)` does, so `1` and `"1"` are one id.
fn id_key(id: &Value) -> String {
    match id {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

struct State {
    client_name: String,
    roots_capable: bool,
    next_ask: u64,
    /// Requests this server sent the client, by id, waiting for their answers.
    asked: HashMap<String, oneshot::Sender<Value>>,
    /// The Ctrl+C of each running tool call, by request id.
    cancels: HashMap<String, mpsc::UnboundedSender<()>>,
}

pub struct McpServer<C, K> {
    deps: Rc<Deps<C, K>>,
    send: Box<dyn Fn(&Value)>,
    state: RefCell<State>,
}

impl<C: Connector, K: Clock> McpServer<C, K> {
    /// A server that sends each message through `send`.
    pub fn new(deps: Rc<Deps<C, K>>, send: impl Fn(&Value) + 'static) -> Rc<Self> {
        Rc::new(McpServer {
            deps,
            send: Box::new(send),
            state: RefCell::new(State {
                client_name: MCP_CLIENT_NAME.to_owned(),
                roots_capable: false,
                next_ask: 0,
                asked: HashMap::new(),
                cancels: HashMap::new(),
            }),
        })
    }

    /// Handles one line of input.
    pub async fn receive(&self, line: &str) {
        let Ok(message) = serde_json::from_str::<Value>(line) else {
            (self.send)(&json!({
                "jsonrpc": "2.0",
                "id": null,
                "error": { "code": -32700, "message": "Parse error" },
            }));
            return;
        };
        let id = message.get("id").filter(|id| !id.is_null()).cloned();
        let Some(method) = message.get("method").and_then(Value::as_str) else {
            // An answer to something this server asked the client.
            let waiter = id.and_then(|id| self.state.borrow_mut().asked.remove(&id_key(&id)));
            if let Some(waiter) = waiter {
                // The asker may have given up waiting.
                let _ = waiter.send(message);
            }
            return;
        };
        let params = message
            .get("params")
            .filter(|params| params.is_object())
            .cloned()
            .unwrap_or_else(|| json!({}));
        let Some(id) = id else {
            if method == "notifications/cancelled"
                && let Some(request) = params.get("requestId")
                && let Some(cancel) = self.state.borrow().cancels.get(&id_key(request))
            {
                // The call ends on its own once the cancel is heard; a send can only fail
                // when it has ended already.
                let _ = cancel.send(());
            }
            return;
        };
        let result = match method {
            "initialize" => self.initialize(&params),
            "ping" => json!({}),
            "tools/list" => {
                let tools: Vec<Value> = TOOLS
                    .iter()
                    .map(|tool| {
                        json!({
                            "name": tool.name,
                            "description": tool.description,
                            "inputSchema": tool.input_schema(),
                        })
                    })
                    .collect();
                json!({ "tools": tools })
            }
            "tools/call" => self.call(&params, &id).await,
            other => {
                (self.send)(&json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32601, "message": format!("Method not found: {other}") },
                }));
                return;
            }
        };
        (self.send)(&json!({ "jsonrpc": "2.0", "id": id, "result": result }));
    }

    fn initialize(&self, params: &Value) -> Value {
        let mut state = self.state.borrow_mut();
        if let Some(name) = params
            .pointer("/clientInfo/name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty())
        {
            state.client_name = name.chars().take(150).collect();
        }
        state.roots_capable = params
            .get("capabilities")
            .and_then(Value::as_object)
            .is_some_and(|capabilities| capabilities.contains_key("roots"));
        let asked = params.get("protocolVersion").and_then(Value::as_str);
        let version = asked
            .filter(|asked| PROTOCOL_VERSIONS.contains(asked))
            .unwrap_or(PROTOCOL_VERSIONS[0]);
        json!({
            "protocolVersion": version,
            // The cockpit's own `ev3 mcp` runs this server as a child and passes on its tools
            // only when it speaks the cockpit's contract (EVƎ Cockpit issue #2).
            "capabilities": {
                "tools": {},
                "experimental": {
                    "ev3-cockpit": { "contract": ev3_client::CONTRACT_VERSION },
                },
            },
            "serverInfo": { "name": "ev3-c", "version": self.deps.version },
        })
    }

    async fn call(&self, params: &Value, id: &Value) -> Value {
        let name = params.get("name");
        let Some(tool) = name
            .and_then(Value::as_str)
            .and_then(|name| TOOLS.iter().find(|tool| tool.name == name))
        else {
            let shown = match name {
                Some(Value::String(name)) => name.clone(),
                Some(other) => other.to_string(),
                None => "undefined".to_owned(),
            };
            return tool_error(format!("Unknown tool {shown}."));
        };
        let args = params
            .get("arguments")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        if let Some(problem) = validate(tool, &args) {
            return tool_error(format!("Invalid arguments for {}: {problem}", tool.name));
        }
        let given = args
            .get("project")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let project = match self.confine(given).await {
            Ok(project) => project,
            Err(message) => return tool_error(message),
        };

        let mut argv = (tool.argv)(&args, &project.to_string_lossy());
        argv.push("--json".to_owned());
        let key = id_key(id);
        let (cancel, cancelled) = mpsc::unbounded_channel();
        let client = {
            let mut state = self.state.borrow_mut();
            state.cancels.insert(key.clone(), cancel);
            Client::Mcp {
                name: state.client_name.clone(),
            }
        };
        let mut io = Captured::default();
        let code = run_cli(
            &argv,
            &mut io,
            &self.deps,
            &mut Interrupts::new(cancelled),
            &client,
        )
        .await;
        self.state.borrow_mut().cancels.remove(&key);

        let printed = io.out.join("\n");
        let Ok(Value::Object(mut value)) = serde_json::from_str::<Value>(&printed) else {
            // A usage problem or a failure before any object: what the command said.
            let said: Vec<&str> = std::iter::once(printed.as_str())
                .chain(io.err.iter().map(String::as_str))
                .filter(|text| !text.is_empty())
                .collect();
            return tool_error(if said.is_empty() {
                format!("ev3 c exited with {code}.")
            } else {
                said.join("\n")
            });
        };
        let notes: Vec<Value> = io
            .err
            .into_iter()
            .filter(|text| !text.trim().is_empty())
            .map(Value::String)
            .collect();
        if !notes.is_empty() {
            value.insert("messages".to_owned(), Value::Array(notes));
        }
        let is_error = value.contains_key("error");
        let value = Value::Object(value);
        let text = serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string());
        json!({
            "content": [{ "type": "text", "text": text }],
            "structuredContent": value,
            "isError": is_error,
        })
    }

    /// The project folder, resolved and checked to lie inside the client's roots.
    async fn confine(&self, given: &str) -> Result<PathBuf, String> {
        let roots = self.roots().await;
        let resolved = resolve(&self.deps.cwd, Path::new(given));
        let real = if resolved.exists() {
            real_path(&resolved)
        } else {
            resolved
        };
        if roots.iter().any(|root| real.starts_with(root)) {
            return Ok(real);
        }
        let listed: Vec<String> = roots
            .iter()
            .map(|root| root.display().to_string())
            .collect();
        Err(format!(
            "{given} lies outside the folders this agent may work in ({}).",
            listed.join(", ")
        ))
    }

    /// The client's roots as local folders, or the working folder when it declares none.
    async fn roots(&self) -> Vec<PathBuf> {
        let fallback = vec![real_path(&self.deps.cwd)];
        if !self.state.borrow().roots_capable {
            return fallback;
        }
        let Some(answer) = self.ask("roots/list", json!({})).await else {
            return fallback;
        };
        let Some(roots) = answer.pointer("/result/roots").and_then(Value::as_array) else {
            return fallback;
        };
        let folders: Vec<PathBuf> = roots
            .iter()
            .filter_map(|root| root.get("uri")?.as_str())
            .filter_map(file_uri_to_path)
            .map(|folder| {
                if folder.exists() {
                    real_path(&folder)
                } else {
                    folder
                }
            })
            .collect();
        if folders.is_empty() {
            fallback
        } else {
            folders
        }
    }

    /// Sends a request to the client and waits for its answer, at most 30 s.
    async fn ask(&self, method: &str, params: Value) -> Option<Value> {
        let (answer, answered) = oneshot::channel();
        let id = {
            let mut state = self.state.borrow_mut();
            state.next_ask += 1;
            let id = format!("ev3-c-{}", state.next_ask);
            state.asked.insert(id.clone(), answer);
            id
        };
        (self.send)(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        match tokio::time::timeout(ROOTS_WAIT, answered).await {
            Ok(Ok(message)) => Some(message),
            _ => {
                self.state.borrow_mut().asked.remove(&id);
                None
            }
        }
    }
}

/// What a tool call's command wrote.
#[derive(Default)]
struct Captured {
    out: Vec<String>,
    err: Vec<String>,
}

impl Io for Captured {
    fn out(&mut self, text: &str) {
        self.out.push(text.to_owned());
    }

    fn err(&mut self, text: &str) {
        self.err.push(text.to_owned());
    }
}

/// Serves MCP on this process's standard input and output until the client closes input.
/// Runs inside a `LocalSet`, which runs the requests side by side.
pub async fn serve_stdio<C, K>(deps: Deps<C, K>) -> i32
where
    C: Connector + 'static,
    K: Clock + 'static,
{
    let server = McpServer::new(Rc::new(deps), |message| {
        let mut out = std::io::stdout().lock();
        // A client that closed its end hears nothing more; the end of input ends the server.
        let _ = writeln!(out, "{message}");
        let _ = out.flush();
    });
    let mut input = BufReader::new(tokio::io::stdin());
    let mut line = Vec::new();
    let mut requests = tokio::task::JoinSet::new();
    loop {
        line.clear();
        match input.read_until(b'\n', &mut line).await {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                let text = String::from_utf8_lossy(&line).trim().to_owned();
                if !text.is_empty() {
                    let server = server.clone();
                    requests.spawn_local(async move { server.receive(&text).await });
                }
                // Forget the requests already answered.
                while requests.try_join_next().is_some() {}
            }
        }
    }
    // A client that sends its requests and then closes its input still gets every answer,
    // as the TypeScript server's event loop gave them.
    while requests.join_next().await.is_some() {}
    0
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::Arc;

    use serde_json::{Value, json};

    use super::*;
    use crate::cockpit::{ClientKind, CockpitError};
    use crate::testing::{
        BRICK_ID, Ending, FakeCockpit, FakeConnector, deps, project, script_run, temp_dir,
    };
    use ev3c_core::brick::fake::TestClock;

    /// EVƎ Cockpit's own MCP tools (`ev3 mcp`); EVƎ C's may not repeat any of them.
    const COCKPIT_TOOLS: [&str; 22] = [
        "list_bricks",
        "connect_brick",
        "disconnect_brick",
        "brick_info",
        "brick_storage",
        "list_files",
        "read_file",
        "write_file",
        "remove_file",
        "remove_paths",
        "make_directory",
        "move_file",
        "deploy_and_run",
        "run_program",
        "stop_program",
        "emergency_stop",
        "program_status",
        "list_telemetry_rows",
        "read_telemetry",
        "clear_bluetooth_list",
        "get_settings",
        "set_setting",
    ];

    struct Setup {
        cockpit: Arc<FakeCockpit>,
        cwd: PathBuf,
        server: Rc<McpServer<FakeConnector, TestClock>>,
        sent: Rc<RefCell<Vec<Value>>>,
        next: RefCell<u64>,
    }

    impl Setup {
        fn new() -> Self {
            let cockpit = FakeCockpit::new();
            let deps = deps(&cockpit);
            let cwd = deps.cwd.clone();
            let sent = Rc::new(RefCell::new(Vec::new()));
            let log = sent.clone();
            let server = McpServer::new(Rc::new(deps), move |message: &Value| {
                log.borrow_mut().push(message.clone());
            });
            Setup {
                cockpit,
                cwd,
                server,
                sent,
                next: RefCell::new(0),
            }
        }

        fn answer(&self, id: &Value) -> Option<Value> {
            self.sent
                .borrow()
                .iter()
                .find(|message| message.get("id") == Some(id) && message.get("method").is_none())
                .cloned()
        }

        async fn request(&self, method: &str, params: Value) -> Value {
            let id = {
                let mut next = self.next.borrow_mut();
                *next += 1;
                json!(*next)
            };
            let line = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
            self.server.receive(&line.to_string()).await;
            self.answer(&id)
                .unwrap_or_else(|| panic!("no answer to {method}"))
        }

        async fn call(&self, name: &str, arguments: Value) -> Value {
            self.request(
                "tools/call",
                json!({ "name": name, "arguments": arguments }),
            )
            .await["result"]
                .clone()
        }

        async fn initialize(&self, params: Value) {
            self.request("initialize", params).await;
        }
    }

    fn text(result: &Value) -> &str {
        result["content"][0]["text"].as_str().unwrap()
    }

    #[tokio::test]
    async fn initialize_answers_with_a_version_it_speaks_and_names_the_server() {
        let s = Setup::new();
        let known = s
            .request(
                "initialize",
                json!({ "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "Claude Code" } }),
            )
            .await;
        assert_eq!(known["result"]["protocolVersion"], "2025-06-18");
        assert_eq!(
            known["result"]["serverInfo"],
            json!({ "name": "ev3-c", "version": "1.2.3" })
        );
        assert_eq!(
            known["result"]["capabilities"]["experimental"]["ev3-cockpit"]["contract"],
            json!(ev3_client::CONTRACT_VERSION),
            "the contract of the cockpit client ev3-c is built with"
        );
        let unknown = s
            .request("initialize", json!({ "protocolVersion": "1999-01-01" }))
            .await;
        assert_eq!(unknown["result"]["protocolVersion"], "2025-11-25");
        assert_eq!(s.request("ping", json!({})).await["result"], json!({}));
        assert_eq!(
            s.request("nothing/here", json!({})).await["error"]["code"],
            -32601
        );
    }

    #[tokio::test]
    async fn initialize_and_tools_list_never_reach_the_daemon() {
        // EVƎ Cockpit's `ev3 mcp` starts this server at its own initialize to list its tools,
        // and promises that starting and listing never start a daemon or connect bricks.
        let s = Setup::new();
        s.initialize(json!({ "protocolVersion": "2025-06-18", "capabilities": { "roots": {} }, "clientInfo": { "name": "Claude Code" } }))
            .await;
        s.request("tools/list", json!({})).await;
        assert!(
            s.cockpit.connects.lock().unwrap().is_empty(),
            "initialize and tools/list connected to the daemon"
        );
    }

    #[tokio::test]
    async fn the_tools_work_on_projects_and_repeat_none_of_evec_cockpits() {
        let s = Setup::new();
        let listed = s.request("tools/list", json!({})).await;
        let tools = listed["result"]["tools"].as_array().unwrap();
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(
            names,
            [
                "build_project",
                "upload_project",
                "run_project",
                "list_project_runs",
                "show_project_run",
                "delete_project_runs"
            ]
        );
        for tool in tools {
            let name = tool["name"].as_str().unwrap();
            assert!(!COCKPIT_TOOLS.contains(&name), "{name} is a cockpit tool");
            let schema = &tool["inputSchema"];
            assert_eq!(schema["type"], "object");
            assert_eq!(schema["additionalProperties"], false);
            assert!(
                schema["required"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("project")),
                "{name} works on a project"
            );
        }
        assert_eq!(
            tools[4]["inputSchema"]["properties"]["run"],
            json!({ "type": "integer", "minimum": 1, "description": "The run number, as list_project_runs gives it." })
        );
    }

    #[tokio::test]
    async fn run_project_runs_the_project_and_answers_with_the_runs_object_as_the_agent_kind_mcp() {
        let s = Setup::new();
        s.initialize(json!({ "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "Claude Code" } }))
            .await;
        project(&s.cwd, "hello");
        script_run(
            &s.cockpit.port(BRICK_ID),
            "hello",
            2,
            Ending {
                record: "end=program\nexit=0\n",
                out: "hi\n",
                returned: None,
                stops: true,
            },
        );
        let result = s.call("run_project", json!({ "project": "hello" })).await;
        assert_eq!(result["isError"], false, "{result}");
        let value = &result["structuredContent"];
        assert_eq!(value["project"], "hello");
        assert_eq!(value["result"]["out"], "hi\n");
        assert_eq!(value["result"]["record"]["exit"], 0);
        assert_eq!(
            &serde_json::from_str::<Value>(text(&result)).unwrap(),
            value
        );
        let connects = s.cockpit.connects.lock().unwrap().clone();
        assert_eq!(connects.len(), 1);
        assert_eq!(connects[0].name, "Claude Code (EVƎ C MCP)");
        assert_eq!(connects[0].kind, ClientKind::Mcp);
    }

    #[tokio::test]
    async fn a_project_outside_the_working_folder_is_refused_when_the_client_declares_no_roots() {
        let s = Setup::new();
        s.initialize(json!({ "capabilities": {} })).await;
        let elsewhere = project(&temp_dir("ev3c-mcp-out-"), "hello");
        let result = s
            .call(
                "build_project",
                json!({ "project": elsewhere.to_string_lossy() }),
            )
            .await;
        assert_eq!(result["isError"], true);
        assert!(
            text(&result).contains("outside the folders this agent may work in"),
            "{}",
            text(&result)
        );
        assert!(s.cockpit.names().is_empty());
        assert!(!elsewhere.join("build").exists(), "nothing was built");
    }

    #[tokio::test]
    async fn with_roots_a_project_inside_one_of_them_is_allowed() {
        let s = Setup::new();
        s.initialize(json!({ "capabilities": { "roots": {} } }))
            .await;
        let root = temp_dir("ev3c-mcp-root-");
        let dir = project(&root, "robot");
        let call = json!({
            "jsonrpc": "2.0", "id": "c1", "method": "tools/call",
            "params": { "name": "build_project", "arguments": { "project": dir.to_string_lossy() } },
        });
        let uri = if cfg!(windows) {
            format!("file:///{}", root.to_string_lossy().replace('\\', "/"))
        } else {
            format!("file://{}", root.to_string_lossy())
        };
        let call = call.to_string();
        let ((), ()) = tokio::join!(s.server.receive(&call), async {
            let ask = loop {
                let found = s
                    .sent
                    .borrow()
                    .iter()
                    .find(|m| m["method"] == "roots/list")
                    .cloned();
                match found {
                    Some(ask) => break ask,
                    None => tokio::task::yield_now().await,
                }
            };
            let answer = json!({ "jsonrpc": "2.0", "id": ask["id"], "result": { "roots": [{ "uri": uri }] } });
            s.server.receive(&answer.to_string()).await;
        });
        let result = s.answer(&json!("c1")).unwrap();
        assert_eq!(result["result"]["isError"], false, "{result}");
        assert!(dir.join("build").join("prog").exists());

        // The working folder is no root of this client.
        project(&s.cwd, "hello");
        let second = json!({
            "jsonrpc": "2.0", "id": "c2", "method": "tools/call",
            "params": { "name": "build_project", "arguments": { "project": "hello" } },
        })
        .to_string();
        let ((), ()) = tokio::join!(s.server.receive(&second), async {
            let ask = loop {
                let found = s
                    .sent
                    .borrow()
                    .iter()
                    .filter(|m| m["method"] == "roots/list")
                    .nth(1)
                    .cloned();
                match found {
                    Some(ask) => break ask,
                    None => tokio::task::yield_now().await,
                }
            };
            let answer = json!({ "jsonrpc": "2.0", "id": ask["id"], "result": { "roots": [{ "uri": uri }] } });
            s.server.receive(&answer.to_string()).await;
        });
        assert_eq!(s.answer(&json!("c2")).unwrap()["result"]["isError"], true);
    }

    #[tokio::test]
    async fn bad_arguments_and_a_cockpit_refusal_come_back_as_tool_errors_not_protocol_errors() {
        let s = Setup::new();
        project(&s.cwd, "hello");
        let missing = s
            .call("show_project_run", json!({ "project": "hello" }))
            .await;
        assert!(text(&missing).contains("\"run\" is required"));
        let zero = s
            .call("show_project_run", json!({ "project": "hello", "run": 0 }))
            .await;
        assert!(text(&zero).contains("\"run\" must be a whole number from 1"));
        let fraction = s
            .call(
                "show_project_run",
                json!({ "project": "hello", "run": 1.5 }),
            )
            .await;
        assert!(text(&fraction).contains("must be a whole number"));
        let extra = s
            .call("build_project", json!({ "project": "hello", "extra": 1 }))
            .await;
        assert!(text(&extra).contains("unknown argument \"extra\""));
        let number = s.call("build_project", json!({ "project": 7 })).await;
        assert!(text(&number).contains("\"project\" must be a string"));
        assert_eq!(s.call("no_such_tool", json!({})).await["isError"], true);

        // JSON's 2.0 is a whole number, as in JavaScript; the run is then looked up.
        let two = s
            .call(
                "show_project_run",
                json!({ "project": "hello", "run": 2.0 }),
            )
            .await;
        assert!(!text(&two).contains("Invalid arguments"), "{}", text(&two));

        *s.cockpit.brick_failure.lock().unwrap() = Some(CockpitError::new(
            "brick_busy",
            "EVƎ C (VS Code) is writing to the brick.",
        ));
        let busy = s
            .call("list_project_runs", json!({ "project": "hello" }))
            .await;
        assert_eq!(busy["isError"], true);
        assert_eq!(
            busy["structuredContent"],
            json!({ "error": { "message": "EVƎ C (VS Code) is writing to the brick.", "code": "brick_busy" } })
        );
    }

    #[tokio::test]
    async fn cancelling_a_run_stops_the_program_as_ctrl_c_does() {
        let s = Setup::new();
        project(&s.cwd, "hello");
        let port = s.cockpit.port(BRICK_ID);
        script_run(
            &port,
            "hello",
            2,
            Ending {
                record: "end=slot\nsignal=15\n",
                out: "",
                returned: None,
                stops: false,
            },
        );
        let call = json!({
            "jsonrpc": "2.0", "id": "r1", "method": "tools/call",
            "params": { "name": "run_project", "arguments": { "project": "hello" } },
        });
        let call = call.to_string();
        let ((), ()) = tokio::join!(s.server.receive(&call), async {
            while !port.with(|brick| brick.running) {
                tokio::task::yield_now().await;
            }
            let cancel = json!({ "jsonrpc": "2.0", "method": "notifications/cancelled", "params": { "requestId": "r1" } });
            s.server.receive(&cancel.to_string()).await;
        });
        assert_eq!(
            port.calls()
                .iter()
                .filter(|call| *call == "stop_program")
                .count(),
            1
        );
        let result = s.answer(&json!("r1")).unwrap();
        let value = &result["result"]["structuredContent"];
        assert_eq!(value["result"]["record"]["end"], "slot");
        assert!(
            value["messages"][0]
                .as_str()
                .is_some_and(|note| note.starts_with("Stopping the program")),
            "what the command told the person comes along: {value}"
        );
    }

    #[tokio::test]
    async fn a_line_that_is_not_json_gets_a_parse_error() {
        let s = Setup::new();
        s.server.receive("{not json").await;
        let sent = s.sent.borrow();
        assert_eq!(sent[0]["error"]["code"], -32700);
        assert_eq!(sent[0]["id"], Value::Null);
    }

    #[test]
    fn whole_numbers_print_as_javascript_prints_them() {
        assert_eq!(number_text(&json!(2)), "2");
        assert_eq!(number_text(&json!(2.0)), "2");
        assert_eq!(number_text(&json!(u64::MAX)), u64::MAX.to_string());
    }
}
