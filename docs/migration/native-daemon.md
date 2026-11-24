# Moving EVƎ C to the native cockpit daemon

EVƎ Cockpit replaces its per-user daemon with one native binary, `ev3-cockpitd`, whose contract
is generated from JSON Schema (contract version 2). The cockpit removes the old daemon and its
hand-written contract in one step at its switch (unit U17 of the cockpit plan), with no shim.
This document records what EVƎ C uses today and what replaces it.

> **Superseded by #13:** the TypeScript backend this document describes is gone. `src/cockpit/`
> and the `@ev3-cockpit/client` tarball no longer exist. The VS Code extension runs the Rust
> program `ev3-c`, which reaches the daemon through the cockpit's Rust crate `ev3-client`
> (`backend/crates/ev3-c/src/daemon.rs`). The record below stays for its history.

The reference is the cockpit's `docs/api-for-consumers.md`, section "Moving to the native daemon:
what changes in the package", and `contract/methods.json`. The column "After the switch" there is
planned, not confirmed.

## Where EVƎ C starts from

EVƎ C never used the Node daemon's client (`CockpitClient`, `WireErrorCode`, `clientPaths`,
`installedLaunch`, launch options). It calls the VS Code extension API v1:
`vscode.extensions.getExtension('99LevelsUp.ev3-cockpit').activate()`, then
`registerConsumer('ev3-c')`. Its types are copied into `src/host/cockpitTypes.ts`. The move
therefore goes from API v1 straight to the native client; the Node daemon stage is skipped.

The cockpit's documentation already says its extension exports nothing, so API v1 may be gone
from current cockpit builds (see "Answers from the cockpit").

## Today → native

| Today (API v1, `CockpitConsumer`) | Native (`CockpitClient`, contract 2) | Used in EVƎ C | Notes |
|---|---|---|---|
| `getExtension(id).activate()`, `apiVersion === 1`, `registerConsumer('ev3-c')` | `CockpitClient.connect({ name, kind: 'addon', version })`; no `daemon` option, the client starts what `install.json` records | `src/extension.ts`, `src/host/cockpitAdapter.ts` `connectCockpit` | Start failures are a `DaemonStartError` with `failure`; a contract mismatch is `failure: 'contract_mismatch'`, not a version check of ours |
| consumer `dispose()` | `client.dispose()` | `src/extension.ts` | Closes the connection and every subscription |
| `listBricks()` → `ApiBrick { signedIn, active, … }` | `client.bricks()` → `ContractBrick { connected, reconnecting?, holders, … }` | `cockpitAdapter.ts` `isBrickOnline`, `statusBar.ts` | `signedIn` becomes `connected`; there is no `active` |
| `getActiveBrickId()`, `onActiveBrickChanged` | none: each client keeps its own active brick (`activeBrick()`, `setActiveBrick()`), and setting it starts foreground telemetry | `resolveBrickId` for the setting `ev3-c.brick = "active"`, `statusBar.ts` | EVƎ C can no longer read the brick picked in the cockpit's UI. EVƎ C does not call `setActiveBrick`, because it shows no telemetry (decision of 22.11.2026) |
| `onBricksChanged` | `client.onBricksChanged` | `cockpitAdapter.ts`, `statusBar.ts` | Same meaning |
| `list(brick, path)` → `FsEntry[]` | `files/list { brick, path }` → `FsEntry[]` | `deploy.ts`, `runFolders.ts`, `runResults.ts` | Same entry shape (`md5` lowercase hex) |
| `exists(brick, path)` | `files/exists` | `deploy.ts`, `runFolders.ts` | |
| `readFile(brick, path)` → bytes | `files/read { path, toLocalPath }` into a private temporary file | `runResults.ts` | Inline reads stop at 64 KiB (since cockpit a7ac92c the daemon refuses a larger file after the brick's first reply); a run's output may be larger (`outputCapKiB`), so EVƎ C always reads to a local file |
| `writeFile(brick, path, bytes)` | `files/write { path, fromLocalPath }` (absolute path) or `{ content }` (base64, at most 4 KiB) | `deploy.ts` (Upload) | Folders are created by the write |
| `makeDirectory(brick, path)` | `files/makeDirectory` | `deploy.ts` | No longer needed before a write |
| `remove(brick, path, { recursive })` | `files/remove` → `RemoveReport { leftBehind }`; several paths under one lease: `files/removePaths` | `runFolders.ts` `deleteRuns`, `runControl.ts` (`.req` cleanup) | `leftBehind` says what a recursive remove left; EVƎ C used to list again to find out |
| `writeFile(.req)` + `runProgram(brick, rbf)` | `programs/deployAndRun { files, runPath }` → `DeployResult { outcome, runSeq, uploaded, skipped }`, one lease | `runControl.ts` `runProject` (Run) | Binary, supervisor and launcher go by `localPath` with `skipIfSameMd5`; `.req` goes inline. Nothing runs → checked inside the lease |
| `runProgram` → `ProgramState { running }` | `DeployResult.outcome`: `started`, `not_seen_running`, `uncertain` | `runControl.ts` | `uncertain`: read `programs/status`, never start again |
| `stopProgram(brick)` | `programs/stop` → `ProgramState` | `runControl.ts`, Stop command | Never waits for a lease; once nothing runs, `endedBy` names the client whose stop ended the run, or `brick` (contract 3) |
| `emergencyStop(brick)` | `programs/emergencyStop` | Emergency Stop command | |
| `isProgramRunning(brick)` | `programs/status` (`waitMs` optional) | `deploy.ts`, `runFolders.ts`, `runControl.ts`, `statusBar.ts` | |
| `onProgramStateChanged(brick, listener)`; ends silently when the session ends | `client.subscribePrograms(brick, listener)` → `{ state, dispose }`; `ProgramState { running, runSeq, endedBy? }` | `cockpitAdapter.ts` `BrickEvents`, `runControl.ts`, `statusBar.ts` | The run's end is `running: false` with `runSeq >=` the run's own; it replaces the "armed" wait. Still ends silently when the brick is signed out |
| no reconnect event; EVƎ C watched `signedIn` | `bricks/reconnected` notification; `client.onReconnected` for a new daemon | `cockpitAdapter.ts` | |
| error `code` (string) | `error.code`, the contract's `ErrorCode`; `data.holder`, `data.candidates`, `data.command` | `errorMessages.ts`, `runControl.ts`, `deploy.ts` | `kind` (error class) is gone; new codes: `brick_busy`, `brick_ambiguous`, `brick_not_found`, `brick_unreachable`, `link_lost`, `needs_person`, `daemon_unavailable`, `outcome_unknown`, `cancelled`, `invalid_request` |
| no questions | `client.onQuestion((q, { signal }) => 'yes' \| 'no' \| 'keep')` | new in `src/host` | An add-on may be asked `sweep` for its own transfers; `signal` aborts when the daemon withdraws the question |
| no cancellation | `request(method, params, { signal })` → `cancelled` | not used yet | For Ctrl+C in `ev3 c run` |
| no log | `log/notice { level, text }` | not used yet | For toasts EVƎ C shows |

Exports EVƎ C does not need: `installedNativeLaunch` and `nativeChannelFiles`/`nativeStateRoot`
(the client reads `install.json` itself), `ensureNativeDaemon`, `decideOnDaemon`,
`packagedDaemon`, `developmentDaemon`, `DAEMON_MANIFEST`. An add-on never replaces the daemon and
never sends `daemon/exit`.

`extensionDependencies` on `99LevelsUp.ev3-cockpit` stays: the cockpit's extension writes
`install.json` when it connects, and without it the first connect can fail with `missing_binary`.

## How EVƎ C reaches the library before it is published

`@ev3-cockpit/client` is not on npm yet. `scripts/stage-cockpit-client.cjs` stages it from a
compiled cockpit checkout (`npm run compile` there) with the cockpit's own `stageClient`, packs it
into `vendor/ev3-cockpit-client.tgz`, and `package.json` depends on that tarball. The tarball is
committed (the library is MIT and the cockpit's repository is public), so a fresh clone and CI
build without the cockpit's sources. Replace the dependency with the published version once it
exists.

Since the cockpit's first pre-release, the vendored tarball is the one the release carries
(`ev3-cockpit-client-<version>.tgz`, now `0.3.0-preview.1`), checked with
`gh attestation verify <file> --repo 99LevelsUp/ev3-cockpit` before it is committed. The staging
script stays for a client change the cockpit has not released yet.

Only `src/cockpit/cockpitLink.ts` imports the library (a unit test guards this), always as `@ev3-cockpit/client`, so the
renames at the switch (`NativeCockpitClient` → `CockpitClient`, the contract at the top level)
touch one file.

## Answers from the cockpit (22.11.2026)

1. **The active brick** cannot be read today: each client keeps its own foreground brick and the
   daemon tells no one else, and the extension exports nothing. The cockpit puts the question to
   its owner (one proposal: `daemon/status` says which brick each client shows). Until then
   `ev3-c.brick = "active"` means the only connected brick; with several, the author sets the
   setting. The rule lives in one function, `resolveBrick` in `src/cockpit/cockpitLink.ts`.
2. **API v1** left the cockpit's local master on 20.11.2026 (cf8c8e6), which is not pushed;
   everything published still carries it, so EVƎ C's `main` works against published cockpit
   builds. The cockpit tells EVƎ C when its master goes to origin and when U17 runs; this
   branch merges at one of those, at the latest with U17. Until then it is neither merged nor
   pushed.
3. **The package** is staged again from `feat/native-daemon` in the cockpit's
   `dist/npm/client`. EVƎ C stages its own copy with `scripts/stage-cockpit-client.cjs`.
4. **Reading files** is not limited to one read per second with the native daemon: it serialises
   transfers per brick and releases each firmware slot. EVƎ C no longer paces its reads
   (`PacedReader` is gone).

## What changed in EVƎ C

- `BrickPort` follows the native contract: `writeFile` takes a local path or a few bytes,
  `removePaths` replaces `remove` and `makeDirectory`, `deployAndRun` replaces `runProgram`,
  `programState` replaces `isProgramRunning`, states carry `runSeq`, and `onBrickEvent` resolves
  once its subscription is live.
- **Run** uploads the binary, the supervisor and the launcher (`skipIfSameMd5`) and the launch
  request (inline) in the one `programs/deployAndRun` that starts the launcher. The run has ended
  when a stopped state arrives with `runSeq` at least the start's own, or a later start shows;
  `not_seen_running` starts the result wait at once, `uncertain` reads `programs/status` and is
  never repeated. States heard before the start returns are kept, so a short run is not missed.
- **Upload** without a run still writes file by file after its own MD5 check, and retries a
  write once only after `not_connected`, `link_lost` or `transport_failed`.
- **Reads** go to a private temporary file (`files/read` with `toLocalPath`), so a run's output
  larger than 64 KiB is read too.
- **Delete Runs** removes the chosen run folders in one `files/removePaths` and lists the runs
  folder again to report what is still there.
- The host connects on first need (and in the background at activation) as
  `EVƎ C (VS Code)`, kind `addon`, and connects afresh after the library reports the daemon
  lost. Start failures and the new error codes have their own messages; `brick_busy` names the
  client holding the brick. Questions for a person go to a VS Code notification.
- **The brick is held by EVƎ C** (`bricks/connect` without `hold`, as the cockpit recommends), so
  it does not go when whoever else connected it lets it go. The hold ends with EVƎ C's
  connection, the library renews it on a new daemon, and naming another brick releases it.
- **A replaced daemon counts starts from 0 again** (the counter lives in its memory), so after a
  reconnect a stopped slot ends the run's wait whatever its `runSeq`. The cockpit takes this to
  its owner: persist the counter, or document the reset.

## Checked against the native daemon

`tests/replay/deploy-and-run.cjs` replays the cockpit's `usb-deploy-and-run-ccc` scenario through
EVƎ C's session, brick hold and port against `ev3-cockpitd` in replay mode; no brick is needed and
the steps are in the script. It passed on 22.11.2026 with daemon and client 0.2.549: the brick
held, the start `started` with `runSeq` 1, the running and stopped states, the folder removed.
## After the cockpit's switch (23.11.2026)

The cockpit's master now carries the native daemon only, and its client library changed:

- `NativeCockpitClient` is `CockpitClient`, and the generated contract is exported at the
  top level (`ErrorCode`, `ContractBrick`, `DeployFile`, …). EVƎ C's link module is now
  `src/cockpit/cockpitLink.ts`; it is still the only one that imports the library.
- `ContractError` has no `kind`; EVƎ C decides on `error.code` as before. Its messages are
  keyed by the contract's `ErrorCode`, so a code the cockpit renames or drops fails EVƎ C's build.
- **The active brick can be read.** `daemon/status` gives each client's `shows`, and
  `daemon/showsChanged` tells every change. `ev3-c.brick = "active"` now means the brick the
  cockpit's VS Code window (a client of kind `vscode`) shows; with no window showing one, the
  only connected brick; with windows showing different bricks, the author sets the setting.
  The status bar follows the window too.
- **`runSeq` survives a daemon replacement** (the daemon keeps it per brick in `runs.json`). Only
  a damaged `runs.json` starts again from 0, so EVƎ C keeps ending the wait on any stopped slot
  after a reconnect.
- The staging script loads the cockpit's `scripts/package-npm.mts` (Node strips the types) and
  refuses a staged client that is not the native one at the top level.
