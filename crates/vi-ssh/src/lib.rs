//! SSH layer for virt-interact: key-based auth (SPEC-006), ssh config
//! (SPEC-007), known_hosts (SPEC-008), command runner (SPEC-017) and
//! tunnels (SPEC-018, v1.1).

pub mod error;
pub mod known_hosts;
pub mod prompt;
pub mod runner;
pub mod secret;
pub mod session;
pub mod sshconfig;

pub use error::{SshError, SshErrorKind};
pub use prompt::{HostKeyPrompt, NoPrompt, Prompter, ScriptedPrompter};
pub use runner::{CommandOutput, CommandRunner, MockRunner};
pub use secret::Secret;
pub use session::{ConnectOptions, SshSession};
pub use sshconfig::{ResolvedHost, SshConfig, resolve};
