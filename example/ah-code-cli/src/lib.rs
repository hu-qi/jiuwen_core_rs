use std::path::PathBuf;

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Agent(String),
    Help,
    Quit,
    Tools,
    Read(String),
    Write {
        path: String,
        content: String,
    },
    Edit {
        path: String,
        old: String,
        new: String,
    },
    Run {
        command: String,
        args: Vec<String>,
    },
    NewSession(String),
    UseSession(String),
    ListSessions,
    History,
    Clear,
}

pub fn parse_input(input: &str) -> Result<Command, String> {
    let input = input.trim();
    if input.is_empty() {
        return Ok(Command::Clear);
    }
    let Some(command) = input.strip_prefix('/') else {
        return Ok(Command::Agent(input.to_string()));
    };

    let (name, rest) = first_word(command);
    match name {
        "help" => Ok(Command::Help),
        "quit" | "exit" => Ok(Command::Quit),
        "tools" => Ok(Command::Tools),
        "history" => Ok(Command::History),
        "clear" => Ok(Command::Clear),
        "list" | "sessions" => Ok(Command::ListSessions),
        "new" => required_arg(rest, "usage: /new <session-id>").map(Command::NewSession),
        "use" => required_arg(rest, "usage: /use <session-id>").map(Command::UseSession),
        "read" => required_arg(rest, "usage: /read <path>").map(Command::Read),
        "write" => {
            let (path, content) = first_word(rest);
            if path.is_empty() || content.is_empty() {
                return Err("usage: /write <path> <content>".to_string());
            }
            Ok(Command::Write {
                path: path.to_string(),
                content: content.to_string(),
            })
        }
        "edit" => {
            let (path, replacement) = first_word(rest);
            let (old, new) = replacement
                .split_once(" => ")
                .ok_or_else(|| "usage: /edit <path> <old> => <new>".to_string())?;
            if path.is_empty() || old.is_empty() || new.is_empty() {
                return Err("usage: /edit <path> <old> => <new>".to_string());
            }
            Ok(Command::Edit {
                path: path.to_string(),
                old: old.to_string(),
                new: new.to_string(),
            })
        }
        "run" => {
            let (program, args) = first_word(rest);
            if program.is_empty() {
                return Err("usage: /run <command> [args...]".to_string());
            }
            Ok(Command::Run {
                command: program.to_string(),
                args: args.split_whitespace().map(str::to_string).collect(),
            })
        }
        _ => Err(format!("unknown command: /{name}; type /help")),
    }
}

fn first_word(input: &str) -> (&str, &str) {
    let input = input.trim_start();
    match input.find(char::is_whitespace) {
        Some(index) => (&input[..index], input[index..].trim_start()),
        None => (input, ""),
    }
}

fn required_arg(value: &str, usage: &str) -> Result<String, String> {
    if value.is_empty() {
        Err(usage.to_string())
    } else {
        Ok(value.to_string())
    }
}

#[derive(Debug)]
pub struct Options {
    pub workspace: PathBuf,
    pub state_dir: Option<PathBuf>,
    pub model: String,
    pub once: Option<String>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            workspace: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            state_dir: None,
            model: "mock".to_string(),
            once: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_input_becomes_agent_task() {
        assert_eq!(
            parse_input("inspect this project").unwrap(),
            Command::Agent("inspect this project".into())
        );
    }

    #[test]
    fn parses_file_commands_without_losing_content() {
        assert_eq!(
            parse_input("/write notes.txt hello world").unwrap(),
            Command::Write {
                path: "notes.txt".into(),
                content: "hello world".into(),
            }
        );
        assert_eq!(
            parse_input("/edit main.rs old text => new text").unwrap(),
            Command::Edit {
                path: "main.rs".into(),
                old: "old text".into(),
                new: "new text".into(),
            }
        );
    }

    #[test]
    fn parses_session_and_shell_commands() {
        assert_eq!(
            parse_input("/new demo").unwrap(),
            Command::NewSession("demo".into())
        );
        assert_eq!(
            parse_input("/run cargo test -p ah-code-cli").unwrap(),
            Command::Run {
                command: "cargo".into(),
                args: vec!["test".into(), "-p".into(), "ah-code-cli".into()],
            }
        );
    }

    #[test]
    fn rejects_unknown_and_incomplete_commands() {
        assert!(parse_input("/wat").is_err());
        assert!(parse_input("/read").is_err());
        assert!(parse_input("/edit file old").is_err());
    }
}
