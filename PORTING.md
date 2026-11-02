# Source and scope

References are restricted to 0xLattice:

- `lattice-node` at `3df9e0a786c7b103a3da88d1c3881efc81706255`, especially
  `Sources/LatticeCtl/{Key,Tx,RPC,Main}.swift` and `LatticeCtlCore/Topology.swift`.
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

Remaining complete Swift CLI scope:

- `wipe`, foreground crash supervision and service role selection.
- `mine` coordinator/worker process lifecycle and declared recipient plans.
- `child` add/remove and remote-author flows; hosted spec/premine/genesis/policy
  files already translate into daemon configuration, but dynamic reconciliation
  currently restarts the node rather than calling the hosted-chain endpoint.
- Systemd unit emission and platform-specific service support.
- End-to-end Swift topology/parser differential coverage and Windows process/ACL support.

## Local lifecycle slice

`init`, `identity`, `topology`, `up --node-binary PATH`, and `down` use the Swift
host directory layout. Initialization preserves existing files, creates a private
raw overlay seed, and builds canonical Nexus genesis using production validation
and Cashew serialization, asserting the pinned genesis CID. Wallet files and
node overlay identity files remain distinct formats.

A per-root advisory lock serializes startup/shutdown. The PID record includes
kernel birth time and executable identity, checked before signalling; unrelated
or reused PIDs are not treated as owned nodes. Linux opens a pidfd before validating process identity and signals through that
handle; unavailable pidfd support fails closed. On macOS a narrow PID reuse race
remains between kernel inspection and signal delivery. Startup failures kill and reap the newly spawned child.
Readiness requires this spawn's appended JSON ready event. Config/spec content
changes restart the node; unchanged starts are idempotent. There is no background
crash supervisor yet. `down` allows 30 seconds before SIGKILL.

The topology model covers the pinned Swift schema and validation, but rejects
unknown nested mining fields more strictly. Unsupported websocket, external
address, serving/acquisition limits, and nonzero finality options fail explicitly
rather than being dropped. Mining declarations are parsed but do not start miners.
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
