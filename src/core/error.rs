use std::process::Output;

/// Errors that can occur while managing a runspace.
///
/// Messages are written for humans first: every error should tell the user
/// (or the agent reading it) what failed and, when possible, how to fix it.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no active runspace matches '{0}'")]
    NotFound(String),

    #[error("there are no active runspaces; create one with `runspace create`")]
    NoRunspaces,

    #[error("multiple runspaces are active; pass a target: {0}")]
    TargetRequired(String),

    #[error("more than one runspace matches '{target}': {matches}")]
    Ambiguous { target: String, matches: String },

    #[error("a runspace named '{0}' already exists")]
    NameTaken(String),

    #[error("invalid name '{name}': {reason}")]
    InvalidName { name: String, reason: String },

    #[error("{0}")]
    Invalid(String),

    #[error("{tool} is required but was not found on PATH\n  install it with: {install}")]
    MissingTool {
        tool: &'static str,
        install: &'static str,
    },

    #[error("could not start {program}: {source}")]
    Spawn {
        program: String,
        source: std::io::Error,
    },

    #[error("{program} failed: {message}")]
    Command { program: String, message: String },

    #[error("cannot talk to display :{display}: {reason}")]
    Display { display: u16, reason: String },

    #[error("{backend} failed to start on :{display}\n{details}")]
    BackendStart {
        backend: String,
        display: u16,
        details: String,
    },

    #[error(
        "visible mode needs a working host X display\n  DISPLAY={display}\n  Runspace opens a nested Xephyr window on your desktop. Start an X session or use invisible mode."
    )]
    NoHostDisplay { display: String },

    #[error("{0}")]
    Unsupported(String),

    #[error("state error: {0}")]
    State(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

impl Error {
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::Invalid(message.into())
    }

    pub fn command_failed(program: &str, output: &Output) -> Self {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = stderr.trim();
        let message = if stderr.is_empty() {
            match output.status.code() {
                Some(code) => format!("exited with status {code}"),
                None => "was terminated by a signal".to_string(),
            }
        } else {
            stderr.to_string()
        };
        Self::Command {
            program: program.to_string(),
            message,
        }
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
