# Test-only SSH keys

**These keys are for automated tests only. Never use them anywhere else.**
They are committed on purpose so that tests are reproducible on every platform.

| File | Type | Passphrase | Used as |
|------|------|------------|---------|
| `server_ed25519` | ed25519 | none | host key of the in-process test server |
| `client_ed25519` | ed25519 | none | client key accepted by the test server |
| `client_enc_ed25519` | ed25519 | `test-passphrase-123` | encrypted client key (TC-037, TC-039) |
| `client_rsa` | RSA 3072 | none | RSA client key (rsa-sha2 signatures) |
| `other_ed25519` | ed25519 | none | a different key, for host key mismatch tests |
| `openssh_hashed_known_hosts` | — | — | `server_ed25519.pub` for `lab.lan` and `[lab.lan]:2222`, hashed by OpenSSH `ssh-keygen -H` |
