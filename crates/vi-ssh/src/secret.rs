//! A secret string (key passphrase) that is never printed and is wiped on
//! drop (SPEC-006, TC-039).

use std::fmt;

/// A passphrase. `Debug` and `Display` show `***`; the bytes are zeroed when
/// the value is dropped.
pub struct Secret(Vec<u8>);

impl Secret {
    pub fn new(s: impl Into<String>) -> Secret {
        Secret(s.into().into_bytes())
    }

    /// The secret text. Do not log or store it.
    pub fn expose(&self) -> &str {
        std::str::from_utf8(&self.0).unwrap_or_default()
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        for b in self.0.iter_mut() {
            // volatile-like: keep the compiler from eliding the wipe
            *b = std::hint::black_box(0);
        }
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(***)")
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("***")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // TC-039
    #[test]
    fn never_printed() {
        let s = Secret::new("hunter2");
        assert_eq!(s.expose(), "hunter2");
        assert_eq!(format!("{s:?}"), "Secret(***)");
        assert_eq!(format!("{s}"), "***");
        assert!(!format!("{:?}", Some(&s)).contains("hunter2"));
    }
}
