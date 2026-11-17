//! virt-interact desktop GUI (SPEC-003, SPEC-015). The binary in `main.rs`
//! only parses the command line and starts [`app::App`].

pub mod app;
pub mod console;
pub mod prompts;
pub mod startup;
pub mod theme;
pub mod view;
pub mod worker;
