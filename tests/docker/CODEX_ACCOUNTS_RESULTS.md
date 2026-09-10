# Codex account and model drift regression

Verified on 2026-09-10 in the existing `debian` container (Debian 13, aarch64).

- Rust suite: 140 tests passed on macOS and Debian.
- CLI regressions cover account switching after external model/reasoning edits, disconnect restoring those edits, and continued rejection of external provider changes.
- API regression covers explicit reapplication after Codex changes the model/reasoning.
- TUI tests cover dedicated Pi Help, clickable account login, compact rendering and client navigation.
- `tests/docker/client_tabs_isolation.py`: real PTY mouse switching passed with independent provider catalogs and no configuration mutation.
- Clippy: all targets/features, warnings denied; formatting checked.

Real retained login validation used `tests/docker/codex_current_login.py` with `CCSW_TEST_INPUT=/root/codex-test` and the Debian binary `/tmp/ccsw-pi-source/target/debug/ccsw`.

Import and activation passed; disconnect restored the original configuration. The input copies and credentials were unchanged. The retained `/root/codex-test` directory was not removed.

The installed Codex rejected `account/rateLimits/read`. The test recorded a refresh error and no cached quota; quota retrieval is **not** verified as successful. Local identity display does not establish remote credential validity or prove a running Codex App switched accounts.
