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

## Simplified ChatGPT provider workflow

The subsequent account-provider change passed 143 Rust tests on macOS and Debian, plus the real PTY client-isolation test. The TUI now exposes ChatGPT Account as a provider and a clickable entry, with Import, File, Use and Back buttons. Help uses the shared Claude modal with Providers / Accounts / Models / Forms sections.

`codex_account_switch.py` imported the retained previous and newly copied local login. Their identities differed. Switching previous → current → previous → current matched each expected auth file and selected `model_provider = "openai"`. Installed Codex recognized the final ChatGPT login via `codex login status`. No quota calls or remote credential validation were performed. Both retained inputs were unchanged.

Default Debian CCSW now has `Current ChatGPT` selected and its live auth matches the new copy. `Previous (revoked)` remains saved for the user's inspection. The old revoked token was not tested with remote requests and cannot be repaired by switching accounts.

Regression tests also verify that reimported fresh credentials are not overwritten by a revoked live copy of the same identity, and that ChatGPT does not inherit third-party model settings from an unmanaged API configuration.
