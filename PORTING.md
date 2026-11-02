# Source and scope

References are restricted to 0xLattice:

- `lattice-node` at `3df9e0a786c7b103a3da88d1c3881efc81706255`, especially
  `Sources/LatticeCtl/{Key,Tx,RPC,Main}.swift` and `LatticeCtlCore/Topology.swift`.
- `Lattice` at `dda2d852429a27be6043599b63bde81530cd9514`.

This new separate local repository provides `lattice-ctl` as a library and
`lattice` as a binary, using sibling `lattice-sdk-rs`. There is no published
crate or remote commit pin yet. The reference node MIT license is preserved.

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
this initial CLI does not reconcile or enforce the complete declarative topology.

Remaining complete Swift CLI scope:

- `init`, `up`, `down`, `wipe`, process supervision, locks, PID/config reconciliation
  and role selection from the full validated topology model.
- Overlay `identity` and managed identity/custody lifecycle.
- `mine` coordinator/worker process lifecycle and declared recipient plans.
- `child` add/remove, policy/premine/genesis-file handling and remote-author flows.
- Systemd unit emission and platform-specific service support.
- Full topology validation/migration, end-to-end Swift CLI comparison, and Windows
  permissions/process behavior.

These are explicit remaining deliverables; this repository does not claim a
complete replacement for the Swift operator front door yet.
