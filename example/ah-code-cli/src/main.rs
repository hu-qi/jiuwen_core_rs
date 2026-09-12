use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use std::sync::Arc;

use ah_code_cli::{Command, Options, parse_input};
use ah_contracts::agent::AgentLoopRuntime;
use ah_contracts::cli::{CliChunk, CliRenderer};
use ah_contracts::effect::Effect;
use ah_contracts::keys::{AGENT_LOOP, CLI_RENDERER, SESSION_MANAGER, SESSIONS, TOOLS};
use ah_contracts::session::{SessionLog, SessionManager};
use ah_contracts::tools::ToolRegistry;
use ah_hub::Context;
use ah_hub::plugin::DynPlugin;
use ah_plugins_agent_control::AgentControlPlugin;
use ah_plugins_agent_loop::AgentLoopPlugin;
use ah_plugins_cli::CliPlugin;
use ah_plugins_mock::MockPlugin;
use ah_plugins_model_backup::ModelBackupPlugin;
use ah_plugins_openai::OpenAiPlugin;
use ah_plugins_session_log::SessionLogPlugin;
use ah_plugins_sysop::SysopPlugin;
use ah_plugins_tools::ToolsPlugin;
use serde_json::{Value, json};

const ORANGE: &str = "\x1b[38;5;208m";
const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";

struct Runtime {
    workspace: PathBuf,
    state_dir: PathBuf,
    model: String,
    _effects: Vec<Effect>,
    renderer: Arc<dyn CliRenderer>,
    tools: Arc<dyn ToolRegistry>,
    agent: Arc<dyn AgentLoopRuntime>,
    manager: Arc<dyn SessionManager>,
    session: Arc<dyn SessionLog>,
    color: bool,
}

impl Runtime {
    fn print(&self, text: &str) {
        if self.color {
            println!("{ORANGE}{text}{RESET}");
        } else {
            println!("{text}");
        }
    }

    fn print_dim(&self, text: &str) {
        if self.color {
            println!("{DIM}{text}{RESET}");
        } else {
            println!("{text}");
        }
    }

    fn render_chunk(&self, chunk: CliChunk) {
        for line in self.renderer.render(&chunk) {
            println!("{line}");
        }
    }

    fn render_events(&self, session: &Arc<dyn SessionLog>, after_seq: u64) {
        for event in session.since(after_seq) {
            for chunk in self.renderer.chunks_from_event(&event) {
                self.render_chunk(chunk);
            }
        }
    }

    async fn invoke_tool(&self, name: &str, arguments: Value) -> Result<Value, String> {
        self.render_chunk(CliChunk::ToolCall {
            name: name.to_string(),
            arguments: arguments.clone(),
        });
        match self.tools.invoke(name, arguments.clone()).await {
            Ok(value) => {
                let text =
                    serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string());
                self.render_chunk(CliChunk::ToolResult {
                    name: name.to_string(),
                    arguments,
                    result: text,
                });
                Ok(value)
            }
            Err(error) => {
                self.render_chunk(CliChunk::ControllerOutput {
                    text: format!("{name}: {error}"),
                });
                Err(error.to_string())
            }
        }
    }

    async fn run_agent(&self, task: &str) {
        let before = self
            .session
            .events()
            .last()
            .map(|event| event.seq)
            .unwrap_or(0);
        let result = self.agent.run_in_session(self.session.clone(), task).await;
        self.render_events(&self.session, before);
        match result.state {
            ah_contracts::agent::AgentRunState::Completed => self.print_dim(&format!(
                "↳ completed · iterations={} · tools={}",
                result.iterations, result.tool_calls
            )),
            _ => self.render_chunk(CliChunk::ControllerOutput {
                text: result
                    .error
                    .unwrap_or_else(|| format!("agent ended with state {:?}", result.state)),
            }),
        }
    }

    async fn execute(&mut self, command: Command) -> bool {
        match command {
            Command::Agent(task) => self.run_agent(&task).await,
            Command::Help => print_help(),
            Command::Quit => return false,
            Command::Clear => {
                if self.color {
                    print!("\x1b[2J\x1b[H");
                    let _ = io::stdout().flush();
                }
            }
            Command::Tools => {
                let mut names = self.tools.names();
                names.sort();
                for name in names {
                    if let Some(tool) = self.tools.get(&name) {
                        println!("  {name:<16} {}", tool.description());
                    }
                }
            }
            Command::Read(path) => {
                if let Ok(value) = self.invoke_tool("read_file", json!({"path": path})).await
                    && let Some(content) = value.get("content").and_then(Value::as_str)
                {
                    println!("\n{content}");
                }
            }
            Command::Write { path, content } => {
                let _ = self
                    .invoke_tool("write_file", json!({"path": path, "content": content}))
                    .await;
            }
            Command::Edit { path, old, new } => {
                let _ = self
                    .invoke_tool(
                        "edit",
                        json!({"path": path, "old_string": old, "new_string": new}),
                    )
                    .await;
            }
            Command::Run { command, args } => {
                if let Ok(value) = self
                    .invoke_tool(
                        "run_shell",
                        json!({"command": command, "args": args, "timeout_ms": 30_000}),
                    )
                    .await
                {
                    if let Some(stdout) = value.get("stdout").and_then(Value::as_str)
                        && !stdout.is_empty()
                    {
                        println!("{stdout}");
                    }
                    if let Some(stderr) = value.get("stderr").and_then(Value::as_str)
                        && !stderr.is_empty()
                    {
                        eprintln!("{stderr}");
                    }
                }
            }
            Command::NewSession(id) => match self.manager.create(&id) {
                Ok(session) => {
                    self.session = session;
                    self.print(&format!("switched to session {id}"));
                }
                Err(error) => self.render_chunk(CliChunk::ControllerOutput { text: error.0 }),
            },
            Command::UseSession(id) => match self.manager.open(&id) {
                Ok(session) => {
                    self.session = session;
                    self.print(&format!("switched to session {id}"));
                }
                Err(error) => self.render_chunk(CliChunk::ControllerOutput { text: error.0 }),
            },
            Command::ListSessions => {
                for id in self.manager.list() {
                    let marker = if id == self.session.id() { '*' } else { ' ' };
                    println!("{marker} {id}");
                }
            }
            Command::History => {
                for event in self.session.events() {
                    println!("{:>4} {:?} {}", event.seq, event.kind, event.payload);
                }
            }
        }
        true
    }
}

fn print_help() {
    println!(
        "commands:\n  <task>                         run the Agent Loop\n  /read <path>                  read a workspace file\n  /write <path> <content>       write a workspace file\n  /edit <path> <old> => <new>   replace text in a workspace file\n  /run <command> [args...]       run a command in the workspace\n  /tools                        list mounted tools\n  /new <id> | /use <id>         create or switch session\n  /sessions                     list sessions\n  /history                      show append-only session events\n  /clear                        clear the terminal\n  /help | /quit                 show help or exit"
    );
}

fn parse_options<I>(mut args: I) -> Result<Option<Options>, String>
where
    I: Iterator<Item = String>,
{
    let mut options = Options::default();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => return Ok(None),
            "--workspace" => {
                options.workspace = PathBuf::from(
                    args.next()
                        .ok_or_else(|| "--workspace requires a path".to_string())?,
                );
            }
            "--state-dir" => {
                options.state_dir = Some(PathBuf::from(
                    args.next()
                        .ok_or_else(|| "--state-dir requires a path".to_string())?,
                ));
            }
            "--model" => {
                options.model = args
                    .next()
                    .ok_or_else(|| "--model requires mock or openai".to_string())?;
                if options.model != "mock" && options.model != "openai" {
                    return Err("--model must be mock or openai".to_string());
                }
            }
            "--once" => {
                options.once = Some(
                    args.next()
                        .ok_or_else(|| "--once requires a task or command".to_string())?,
                );
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(Some(options))
}

fn boot(options: Options) -> Result<Runtime, Box<dyn std::error::Error>> {
    let workspace = std::fs::canonicalize(&options.workspace)?;
    let state_dir = options
        .state_dir
        .unwrap_or_else(|| workspace.join(".agent-harness-cli"));
    std::fs::create_dir_all(state_dir.join("sessions"))?;

    let ctx = Context::new();
    let model: DynPlugin = if options.model == "openai" {
        Arc::new(OpenAiPlugin::lazy())
    } else {
        Arc::new(MockPlugin)
    };
    let plugins: Vec<DynPlugin> = vec![
        model,
        Arc::new(ToolsPlugin),
        Arc::new(SysopPlugin::new(&workspace)),
        Arc::new(SessionLogPlugin::new(
            state_dir.join("default.jsonl"),
            state_dir.join("sessions"),
        )),
        Arc::new(AgentControlPlugin),
        Arc::new(ModelBackupPlugin::new(Vec::new())),
        Arc::new(CliPlugin),
        Arc::new(AgentLoopPlugin::new(8).with_timeout(Some(120_000))),
    ];
    let effects = ctx.mount_all(plugins)?;
    let renderer = ctx
        .service::<dyn CliRenderer>(&CLI_RENDERER)
        .ok_or("cli renderer seam missing")?;
    let tools = ctx
        .service::<dyn ToolRegistry>(&TOOLS)
        .ok_or("tools seam missing")?;
    let agent = ctx
        .service::<dyn AgentLoopRuntime>(&AGENT_LOOP)
        .ok_or("agent loop seam missing")?;
    let manager = ctx
        .service::<dyn SessionManager>(&SESSION_MANAGER)
        .ok_or("session manager seam missing")?;
    let session = ctx
        .service::<dyn SessionLog>(&SESSIONS)
        .ok_or("session log seam missing")?;

    Ok(Runtime {
        workspace,
        state_dir,
        model: options.model,
        _effects: effects,
        renderer,
        tools,
        agent,
        manager,
        session,
        color: io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none(),
    })
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let Some(options) = parse_options(std::env::args().skip(1))? else {
        println!("ah-code — a Claude Code-style agent-harness example");
        println!(
            "usage: cargo run -p ah-code-cli -- [--workspace PATH] [--model mock|openai] [--once TASK]"
        );
        print_help();
        return Ok(());
    };

    let once = options.once.clone();
    let mut runtime = boot(options)?;
    runtime.print(&format!(
        "ah-code · model={} · workspace={}",
        runtime.model,
        runtime.workspace.display()
    ));
    runtime.print_dim(&format!(
        "state: {} · type /help for commands",
        runtime.state_dir.display()
    ));

    if let Some(input) = once {
        let command = parse_input(&input).map_err(io::Error::other)?;
        runtime.execute(command).await;
        return Ok(());
    }

    print_help();
    let stdin = io::stdin();
    loop {
        if runtime.color {
            print!("{ORANGE}❯{RESET} ");
        } else {
            print!("❯ ");
        }
        io::stdout().flush()?;
        let mut line = String::new();
        if stdin.read_line(&mut line)? == 0 {
            break;
        }
        match parse_input(&line) {
            Ok(command) => {
                if !runtime.execute(command).await {
                    break;
                }
            }
            Err(error) => runtime.render_chunk(CliChunk::ControllerOutput { text: error }),
        }
    }

    Ok(())
}
