# Source and scope

References are restricted to 0xLattice:

- `lattice-node` at `3df9e0a786c7b103a3da88d1c3881efc81706255`, especially
  `Sources/LatticeCtl/{Key,Tx,RPC,Main,Child,Mine}.swift` and `LatticeCtlCore/Topology.swift`.
- `Lattice` at `dda2d852429a27be6043599b63bde81530cd9514`.

This separate repository provides `lattice-ctl` as a library and
`lattice` as a binary, using sibling `lattice-sdk-rs`. Cargo dependencies use the sibling checkout layout; crates.io releases are not provided. The reference node MIT license is preserved.

## Current behavior and evidence

Wallet generation, public display, secure key loading, local JSON RPC/status,
nonce/fee discovery, pending-transaction refusal, offline drafting, and signed
submission are implemented. Five authoring shapes preserve Swift action order,
including a separate fee debit: send, deposit, receipt, withdrawal and child
commitment. The SDK's receipt CID check prevents an unrelated transaction being
reported as successfully submitted.

`python3.12 tools/swift-check.py` checks 80 actual Swift cases across fixed fixture
seeds, zero/nonzero fees, root/child paths and UInt64 nonce boundaries. It compares
public key/address, body CID and signing preimage exactly; Swift verifies the Rust
signature and Rust verifies the Swift signature. Each transaction CID is checked
against its own actual signature. Native Swift signatures varied between runs,
so exact signature-byte equality is not asserted. The Swift harness uses unchanged
pinned Lattice/Crypto/Multikey with action assembly matching the CLI source; it
does not execute the full Swift argument parser or process supervisor.
`tests/swift-evidence.json` records source/corpus hashes and reference revisions.

Key tests check mode 0600, no overwrite, no secret in stdout, rejection of
permissive permissions, symlinks and inconsistent public/private fields. Action
tests reject unrepresentable amounts, malformed paths, child edges and CIDs.
`tools/node-integration.py` runs the actual Rust CLI and disposable localhost
Rust daemon for a signed transfer, default nonce/relay fee discovery, refusal of
an accidentally repeated pending spend, mining, balances and bootstrap-free
restart with rotated cookie. Its immutable binary/source hashes and observations
are in `target/node-integration/report.json`. The publicly known RFC 8032 fixture
key is used only for disposable test data; no real wallet is read.

## Deliberate limits and differences

The CLI is stricter than Swift key loading: the stored public key and address
must match the private key, and symlinks are rejected. On missing account content,
automatic nonce lookup fails instead of inferring zero from 404. Pruned or
unavailable state can return that same status. RPC only accepts literal loopback
IPs, not arbitrary HTTPS endpoints or DNS names. Windows ACL support is pending.

Transaction authoring does not promise successful admission: current state,
parent receipts, policy, limits and signatures are checked by the node. The
five-shape Swift oracle does not prove cross-chain exchange execution or full
host operational parity. Runtime topology membership is decided by the node;
the CLI validates the declarative topology before startup, and the daemon resolves hosted chains.

Remaining parity and platform-specific coverage:

- Additional operator conveniences for removing child configuration; the pinned
  Swift child command itself exposes create/host/commit, not remove.
- End-to-end systemd execution/cgroup tests and platform-specific service support.
- End-to-end Swift topology/parser differential coverage and Windows process/ACL support.

## Local lifecycle slice

`init`, `identity`, `topology`, `up --node-binary PATH`, and `down` use the Swift
host directory layout. Initialization preserves existing files, creates a private
raw overlay seed, and builds canonical Nexus genesis using production validation
and Cashew serialization, asserting the pinned genesis CID. Wallet files and
node overlay identity files remain distinct formats.

A per-root advisory lock serializes startup/shutdown. The PID record includes
kernel birth time and executable identity, checked before signalling; unrelated
or reused PIDs are not treated as owned nodes. Linux records kernel boot identity plus process start ticks, opens a pidfd before
validating that identity, and signals through the handle; unavailable pidfd support fails closed. On macOS a narrow PID reuse race
remains between kernel inspection and signal delivery. Startup failures kill and reap the newly spawned child.
Readiness requires this spawn's appended JSON ready event. Explicit `up` restarts on config/spec content changes; unchanged starts are
idempotent. Child commands apply hosted configuration through the running node. Detached `up` has no crash supervisor; `up --foreground` owns restarts until signalled. `down` allows 30 seconds before SIGKILL.

The topology model covers the pinned Swift schema and validation, but rejects
unknown nested mining fields more strictly. Unsupported websocket, external
address, serving/acquisition limits, and nonzero finality options fail explicitly
rather than being dropped. `up` does not start miners; use the explicit `mine` commands.
Overlay listening defaults to loopback; use `--listen-host 0.0.0.0` explicitly
for external peers. Absent peer configuration uses pinned public bootstrap peers;
`init --no-default-peers` creates an isolated explicit empty list.

`python3.12 tools/lifecycle-integration.py` copies built CLI/node binaries into
an isolated temporary directory and verifies canonical initialization, concurrent
idempotent starts, authenticated status, stop/restart, cookie rotation, persistent
identity, and changed-topology restart. It sets `LATTICE_FORBID_DEFAULT_PEERS=1`.
The report contains binary hashes at `target/lifecycle-integration.json`. Unit
tests cover private/exclusive initialization, managed symlink rejection, stale
birth identity without signalling the live test process, and unsupported topology.
These checks do not claim full Swift lifecycle parity or lattice-formal coverage
for OS process control.

Only initialization creates a missing overlay key. Startup fingerprints include
its derived public key so replacing the key restarts a live daemon. Missing keys
fail closed until restored. Initialization installs nested Git ignore guards in
managed identity/state/runtime/log directories before generating secrets; these
cannot remove secrets already tracked by Git. The lifecycle harness checks these
exclusions, missing-key refusal and live identity replacement.

## Mining lifecycle slice

`mine start/run/stop/status` uses `lattice-mining-rs` Coordinator, HttpNodeClient,
CpuWorker and ProcessWorker directly. No duplicate nonce coordinator is introduced.
Declared recipients and minimum-work requests use the shared template parser;
recipient addresses additionally require canonical standard CIDs and paths must
belong to the declared hosted tree. Worker count is bounded at 1024. CPU workers
are in-process rather than Swift's separate default `lattice-miner` executable.
External workers use the production bounded JSON process-worker protocol.

Each round observes a positive template expiry using the same plan as the real
round. Its deadline is `(expiry + min(longest completed round, expiry)) * multiplier`.
Completed failure/deadline rounds do not increase the measured bound. Parent
acceptance applies declared pacing; child-only submission does not start a hold.
Retries, probes, pacing and active work respond to SIGINT/SIGTERM; worker teardown
uses the shared coordinator cancellation and process-group guard. The CLI retries
node/worker failures without creating overlapping rounds. Cookie files are read
per request, allowing a node restart without restarting the miner.

A separate exclusive miner-owner lease prevents concurrent foreground/background
loops, and miner PID records reuse kernel birth/executable ownership checks.
Status displays the currently configured plan; edits apply after stop/start.
Standalone `mine start` has no crash supervisor; emitted systemd units run `mine run` under the service manager's restart policy. Forced SIGKILL of the
miner bypasses Rust destructors, so process-worker descendants cannot be promised
cleanup in that case. Graceful cancellation is exercised, including a wedged
process worker. Mining uses no private wallet key.

`python3.12 tools/mining-integration.py` copies immutable CLI/node binaries into
an isolated temporary host with public peers forbidden. It verifies actual CPU
reward delivery to a generated disposable address, minimum-work plans, duplicate
start refusal, cookie recovery across node restart, pacing/offline/busy-CPU
cancellation, process-worker descendant teardown, and foreground single-round
operation. Binary hashes are recorded in `target/mining-integration.json`.
Unit tests cover plan refusal and deadline clamping. This does not establish full
Swift process supervision equivalence or a formal proof of OS lifecycle behavior.

## Child lifecycle and authoring slice

`child create PATH` builds or loads a complete ChainSpec and validates its
premine, creator transaction signatures/path/coin restrictions, supplied policy
module CIDs and WASM ABI/entrypoints before creating artifacts. It directly uses
`lattice-node`'s exported pure `hosted_config::check_premine` and transaction
wire decoder; admission still runs through the real node. This adds the node
library's native SQLite/runtime dependency graph to the CLI build. Modules are
validated with `lattice-wasm`; no runtime validation or consensus gate is bypassed.

`child host PATH` declares an existing child, and `child apply` applies the current
hosted set. Live changes use `/hosted-chains`, preserving the owned daemon PID.
Unrelated node configuration or overlay identity changes require `up` first.
Preflight/duplicate failures preserve existing topology/artifacts. Once artifacts
are prepared, a private `run/pending-topology.json` records desired topology before
any live apply request. After that point failures preserve both intent and
artifacts, because a timeout or record-write failure can follow successful node
application. `child apply` retries that intent and finalizes `lattice.json`; `up`
also consumes pending intent and finalizes it after the configured node is ready.
New create/host operations refuse to replace an unresolved pending operation.
A lost response leaves live state uncertain, but not missing its referenced files.
RPC application and filesystem persistence are not one distributed transaction.
Preparation crashes before the intent is recorded can leave unreferenced artifacts
requiring inspection; this is not a general transactional filesystem guarantee.
Configuring a stopped host writes desired state for its next startup.

`child commit PATH --key FILE` uses the pinned `ChildCommitStep` decisions: build
only when the parent enters a new state, retry the same parent commitment until
admitted, keep pushing a carried child, and finish only on `tree`. It uses SDK
nonce/fee discovery, signing and content-bound submission. `--parent-rpc` plus
`--parent-cookie` selects a separate literal-loopback parent endpoint, allowing a
local author daemon connected to another parent daemon. Arbitrary remote URLs
remain outside the operator CLI's local-only RPC policy.

An exclusive per-child lock and private durable journal preserve the candidate,
parent state, signed transaction, acceptance and completion. Interrupted commands
resume the same signed content/nonce. An acknowledged transaction missing from
its parent's mempool is retried unchanged while its nonce remains unused. The
journal stores signatures/public data, never the private key. SIGINT/SIGTERM and
`--timeout-seconds` retain it for resumption. After a completed invocation, a new
invocation deliberately authors the next block. Cross-chain execution and proof
admission remain node decisions, not CLI journal assertions.

`tools/child-integration.py` exercises two actual localhost daemons, invalid and
duplicate create refusal, premine creation, live PID-preserving reconciliation,
separate author/parent commitment, interrupted author-command restart with parent cookie rotation while the author
daemon remains running, exact saved signed-content reuse, one parent nonce increment, child
weighing and premine credit, hosting and apply. Evidence with immutable binary
hashes is in `target/child-integration.json`. This is not a full Swift child-loop
oracle or a lattice-formal proof of filesystem/RPC crash atomicity.

The default integration scenario leaves the author daemon running across the
parent restart and requires it to receive the carrier through the node's peer
reconnection path. The explicit `--restart-author` variant additionally checks
whole-host restart retention. `--keep` retains disposable diagnostics for a failed
run; those temporary files include test-only private keys and must not be published.

`tools/child-recovery-integration.py` forces a real successful hosted apply followed
by a node-config record write failure, then verifies preserved artifacts/intent and
`child apply` recovery. A localhost mock records application but withholds its HTTP
response beyond the production 30-second deadline; retry converges without
recreating artifacts. It also checks `up` consumes pending desired topology.
No production failpoint or weakened timeout is used. Evidence is recorded in
`target/child-recovery-integration.json` with immutable binary hashes.

## Foreground supervision, service output and wipe

`up --foreground --node-binary PATH` holds one supervisor lease per host. It
starts/adopts the owned daemon, retains its original kernel identity in memory,
and restarts after exit. PID-file loss does not authorize a second startup or lose
shutdown ownership. A failed initial validation never adopts or stops an existing
daemon; cleanup preserves a replacement record it did not adopt. SIGINT/SIGTERM
stops reconciliation and gracefully stops the owned node.
`down` stops the daemon but a surviving supervisor will restart it, matching the
pinned Swift behavior; stop the foreground supervisor to keep the host down.
Configuration changes apply through explicit `up`/child commands or the next
crash restart, not a continuous configuration watcher.

`emit-systemd` prints tree and optional miner units without installing them,
creating accounts or invoking a service manager. Units run foreground commands,
use `KillMode=mixed` and allow 90 seconds for shutdown. They intentionally omit
`ExecStop=... down`: signalling the supervisor avoids a down-versus-respawn race.
Command paths are absolute, quoted, percent-escaped and reject control characters;
the `:` command prefix disables environment expansion. Syntax follows the
[systemd.service command-line rules](https://www.freedesktop.org/software/systemd/man/latest/systemd.service.html#Command%20lines).
Node output remains in the host log; supervisor/miner output is available to the
service manager. Actual unit installation and systemd process/cgroup execution
are outside the local validation performed here.

`wipe` rejects a running supervisor/node and acquires the actual NodeStore writer
lock even when the PID record is missing. It refuses symlinked managed storage,
atomically detaches only `chains/Nexus`, and removes that detached directory while
holding the locks. Identity, topology, child specs, pending intent and author
journals remain. A deletion failure reports the detached `.wipe-*` directory for
inspection; this is not a rollback promise. Locks cover cooperating processes;
this does not claim immunity to hostile same-user filesystem races or nested
mount points.

`tools/supervision-integration.py` uses disposable host directories and copied
binaries to check crash restart, duplicate supervisor refusal, graceful shutdown,
writer-lock refusal with a missing PID file, preservation/idempotence/symlink
checks for wipe, clean boot after wipe, and conditional service output with
special path characters. It never installs a service or wipes user data.
The report is `target/supervision-integration.json`; generated synthetic unit
fixtures live in `target/systemd-fixtures` for `systemd-analyze verify`.

The generated tree/miner fixtures passed `systemd-analyze verify` in a disposable
Ubuntu 24.04 container; no units were installed or started. The Linux boot-identity/pidfd module and its tests were cross-compiled for Linux;
runtime execution remains for Linux CI. The local daemon harness here runs on
macOS. Linux PID records from older builds without boot identity no
longer match and cannot authorize signalling; perform a controlled shutdown with
the original service/tool before upgrading a running host.

## Tree-wide health presentation

Default `status` now follows pinned Swift `Tree.swift` Status (lines 413–438) and
its health helper (line 136): check recorded process ownership, enumerate Nexus
and declared hosted children, and present phase, height, tip prefix and mempool
count from `/health`. It never reads or requires the node cookie. The Rust SDK's
public health client supplies bounded, typed reads, with decimal heights retained
without narrowing to machine or JavaScript numbers. Requested/returned chain paths
must agree. This is an operator snapshot, not admission/proof validation.

`status --json` returns a structured tree report. Explicit global `--chain PATH`
retains the prior authenticated single-chain `/status` JSON operation; no default
chain option is implicitly mistaken for an explicit selection. Existing machine
harnesses now select `--chain Nexus` for those single-chain fields.

The tree probe uses at most 16 concurrent requests, each bounded at five seconds
and 64 KiB, with a five-second overall budget. This is stricter than Swift's
sequential per-path timeout. Completed rows remain visible when another request
stalls. Missing/stale ownership yields down rows without RPC; ownership inspection
errors are reported as unverified rather than claiming ownership or hiding health.
Pending desired configuration is labelled while rows use committed topology.
Terminal presentation replaces control characters in untrusted strings.

`tools/status-integration.py` verifies actual root and awaiting-genesis child
health with the cookie removed, default table and structured output, explicit-chain
compatibility, encoded paths, no Authorization header, partial success within the
five-second bound, wide heights, unreachable endpoint behavior and stale process
ownership refusal. Localhost mocks cover stalled responses; no external nodes or
wallets are used. Binary hashes appear in `target/status-integration.json`.
Windows keeps explicit-chain RPC access; owned tree inspection requires Unix.
