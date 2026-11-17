//! Running remote commands behind a trait (SPEC-017), so backends can be
//! tested without a real host ([`MockRunner`]).

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Mutex;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite};
use vi_core::model::ConsoleTarget;

use crate::error::SshError;
use crate::prompt::BoxFuture;

/// Result of one remote command.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CommandOutput {
    /// `None` if the server sent no exit status (e.g. killed by a signal).
    pub exit_status: Option<u32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl CommandOutput {
    pub fn ok(stdout: impl Into<Vec<u8>>) -> CommandOutput {
        CommandOutput {
            exit_status: Some(0),
            stdout: stdout.into(),
            stderr: Vec::new(),
        }
    }

    pub fn failed(code: u32, stderr: impl Into<Vec<u8>>) -> CommandOutput {
        CommandOutput {
            exit_status: Some(code),
            stdout: Vec::new(),
            stderr: stderr.into(),
        }
    }

    pub fn success(&self) -> bool {
        self.exit_status == Some(0)
    }

    pub fn stdout_text(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }

    pub fn stderr_text(&self) -> String {
        String::from_utf8_lossy(&self.stderr).into_owned()
    }
}

/// A byte stream to a console on the host (SPEC-018).
pub trait ConsoleStream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> ConsoleStream for T {}

/// Something that runs a command line on a remote host.
pub trait CommandRunner: Send + Sync {
    fn run<'a>(&'a self, command: &'a str) -> BoxFuture<'a, Result<CommandOutput, SshError>>;

    /// Open a stream to `target` as seen from the host, for a console
    /// tunnel (SPEC-018). `origin` is the local client being forwarded.
    /// Runners without forwarding refuse.
    fn open_stream<'a>(
        &'a self,
        target: &'a ConsoleTarget,
        _origin: SocketAddr,
    ) -> BoxFuture<'a, Result<Box<dyn ConsoleStream>, SshError>> {
        Box::pin(async move {
            Err(SshError::Forward {
                target: target.to_string(),
                reason: "this connection cannot forward".into(),
            })
        })
    }
}

/// Canned responses per exact command line; records every command it gets.
/// Unknown commands give exit status 127.
#[derive(Default)]
pub struct MockRunner {
    responses: HashMap<String, CommandOutput>,
    delays: HashMap<String, Duration>,
    log: Mutex<Vec<String>>,
}

impl MockRunner {
    pub fn new() -> MockRunner {
        MockRunner::default()
    }

    pub fn with(mut self, command: &str, output: CommandOutput) -> MockRunner {
        self.responses.insert(command.to_string(), output);
        self
    }

    /// Delay the response to `command` (for timeout tests).
    pub fn with_delay(mut self, command: &str, delay: Duration) -> MockRunner {
        self.delays.insert(command.to_string(), delay);
        self
    }

    /// All commands run so far, in order.
    pub fn commands(&self) -> Vec<String> {
        self.log.lock().expect("lock").clone()
    }
}

impl CommandRunner for MockRunner {
    fn run<'a>(&'a self, command: &'a str) -> BoxFuture<'a, Result<CommandOutput, SshError>> {
        self.log.lock().expect("lock").push(command.to_string());
        let out = self.responses.get(command).cloned().unwrap_or_else(|| {
            CommandOutput::failed(127, format!("mock: unknown command: {command}\n"))
        });
        let delay = self.delays.get(command).copied();
        Box::pin(async move {
            if let Some(d) = delay {
                tokio::time::sleep(d).await;
            }
            Ok(out)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_runner_records_and_answers() {
        let m = MockRunner::new().with("virsh version", CommandOutput::ok("x"));
        assert_eq!(m.run("virsh version").await.unwrap().stdout_text(), "x");
        let o = m.run("rm -rf /").await.unwrap();
        assert_eq!(o.exit_status, Some(127));
        assert_eq!(m.commands(), vec!["virsh version", "rm -rf /"]);
    }
}
