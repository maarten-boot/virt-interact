//! Core types for virt-interact: configuration (SPEC-004/005), theme and
//! contrast (SPEC-001), paths and command line, logging (SPEC-016),
//! inventory model (SPEC-009/010).

pub mod config;
pub mod contrast;
pub mod logging;
pub mod model;
pub mod paths;

/// Application name, used for config/cache/log directories (SPEC-004).
pub const APP_NAME: &str = "virt-interact";

/// Application version from Cargo.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_name_is_stable() {
        assert_eq!(APP_NAME, "virt-interact");
        assert!(!VERSION.is_empty());
    }
}
