# Gate 4 offline qualification

Run the local gate from a clean checkout on macOS arm64:

```sh
python3 scripts/check-gate-4.py --codex-binary /absolute/path/to/codex
```

This selects an actual installed Codex executable for the existing discovery,
saved-call and reply-continuation fixtures. They use isolated private profiles
and scripted loopback Responses peers; they do not use a model account or
perform inference. No provider API keys, public ingress or live calls are used.
Required tools are Git with the pinned historical commits, Rust from the
repository toolchain, Python 3.11+, Node 22+ and npm. Dependency preparation and
package installation may download pinned source and build dependencies.

This lane disables debug symbols and incremental compiler caching to bound
scratch-disk growth. Debug assertions remain enabled. The report records this
build profile; it is not release-package or performance qualification.

Use `--list` to inspect commands and per-phase budgets without running them.
The runner prepares dependencies before builds, installs and tests the language
SDKs, runs strict workspace lint and Rustdoc, all-feature workspace tests,
explicit media-pressure and installed-host checks, upstream regressions, native
capacity, historical recovery and prior process/example checks. Each required
phase must pass. The seven normally ignored workspace checks are matched by name
and executed by their explicit follow-up phases; an unexpected omission fails.

`--source-only` is a separate CI scope. It omits the six external-host checks
and reports `passed-source-only` if its included phases pass. It does not
qualify the full offline gate or close host acceptance. The CI workflow is
configuration until an actual run is recorded; local execution does not attest
a remote CI pass.

## Evidence and failures

Every invocation writes a private `.local/gate-4-checks/<timestamp>/result.json`
and phase logs. The report records the source revision, non-document source
hashes, tool versions, explicit commands, budgets, phase status, Rust summaries,
failures and work not run. Documentation can be updated during a run; changing
qualified code, contracts, SDK distribution or other source fails the run.
Status is written before each phase and after it completes. A saved `running`
status alone is not evidence of a live process.

The runner fails on missing tools, command errors, deadline expiry, unexpected
ignored tests, source changes or cleanup errors. It stops at the first failed
phase and does not automatically retry uncertain work or expand a budget. Keep
failed reports alongside corrective evidence. Each phase has a finite budget;
the baseline build phases allow their documented dependency/build budgets.

A private supervisor keeps each phase's process-group leader alive until the
driver signals and reaps it. This prevents signalling a reused group ID. A
command leaving a live inherited child is failed even when cleanup succeeds.
Timeout and interruption stop the owned inherited group without signalling an
unrelated worker. This is process ownership, not containment of deliberately
detached sessions; a forcibly killed top-level runner requires its external
supervisor to clean up. Actual host adapters retain their own tested lifecycle
ownership.

Raw logs and completion markers remain private. Child phases receive a limited
build environment without inherited provider/model API tokens. Installed-host
fixtures isolate model configuration and do not load the user's account.

The [numbered plan](connect-gate-4-implementation.html#G4-031) remains the source
of completion status. Passing this lane does not prove real-model reasoning,
provider/carrier behavior, installed release packaging, all product journeys or
whole Gate 4 acceptance. The separate live and installed-candidate requirements
remain intact.
