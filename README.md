# lattice-ctl-rs

A separate Rust operator CLI and reusable library for the 0xLattice platform.
The `lattice` executable supports private wallet key files, authenticated local
RPC/status, and all five ordinary transaction authoring shapes from pinned
Swift `LatticeCtl/Tx.swift`. It uses the Rust SDK for signing, typed nonce/fee
reads and content-bound submission. No app-chain implementation is included.

```sh
cargo build --locked --bin lattice
./target/debug/lattice key generate --out wallet.json
./target/debug/lattice --rpc http://127.0.0.1:8080 --cookie data/.cookie status
./target/debug/lattice --rpc http://127.0.0.1:8080 --cookie data/.cookie \
  tx send --key wallet.json --to ADDRESS --amount 10
```

RPC defaults to `127.0.0.1:8080`, or the `rpc` field in `--root/lattice.json`.
The default cookie is `--root/chains/Nexus/.cookie`, matching Swift's host layout.
Use explicit `--rpc` and `--cookie` for a Rust daemon configured with another data
directory. `--chain Nexus/Alpha` scopes reads and authored transactions.

`key generate` never overwrites files and creates mode 0600. `key show --key FILE`
prints only address/public key. Signing checks permissions and that all three
key fields agree; loading a symlink is refused. Secure key file handling currently
requires Unix. Windows key operations fail explicitly pending ACL support.

Transaction commands:

- `tx send --to ADDRESS --amount N`
- `tx deposit --swap-nonce N --demand N --lock N`
- `tx receipt --swap-nonce N --demand N --demander ADDRESS --directory EDGE`
- `tx withdraw --swap-nonce N --demand N --demander ADDRESS --amount N [--to ADDRESS]`
- `tx commit-child --directory EDGE --block CID --parent-state CID [--previous VALUE]`

Each requires `--key FILE`. Defaults query the committed nonce and relay fee;
a pending first-signer transaction refuses automatic nonce selection. Explicit
`--nonce` enables intentional replacement; the node decides fee/admission rules.
`--dry-run` prints the signed submission payload without submitting. Supplying
both `--nonce` and `--fee` makes dry-run independent of the node/cookie. Generated
signatures and admission acknowledgements do not establish chain inclusion.

`rpc /PATH` prints a JSON GET response. `--method POST|DELETE` and `--body FILE`
expose explicit operator RPC calls. Paths cannot redirect to another authority;
operator RPC requires a literal loopback IP and disables HTTP redirects. Cookie
contents are re-read for every request to tolerate daemon restart rotation.
Requests have 30-second deadlines and bounded JSON responses.

See [PORTING.md](PORTING.md) for validation, deliberate safety differences, and
the remaining host lifecycle/mining/child-management CLI work.

Local host lifecycle (Linux/macOS):

```sh
./target/debug/lattice --root "$HOME/.local/share/lattice" init --no-default-peers
./target/debug/lattice --root "$HOME/.local/share/lattice" identity
./target/debug/lattice --root "$HOME/.local/share/lattice" topology
./target/debug/lattice --root "$HOME/.local/share/lattice" up --node-binary ../lattice-node-rs/target/debug/lattice-node
./target/debug/lattice --root "$HOME/.local/share/lattice" status
./target/debug/lattice --root "$HOME/.local/share/lattice" down
```

`up` is serialized and idempotent; config changes restart the owned process.
It currently starts a detached daemon without crash supervision. Service units, crash supervision and wipe remain pending; see `PORTING.md` for precise differences.

Keep host data outside source checkouts. Initialization also installs Git ignore
guards inside managed identity/state/runtime/log directories; these do not remove
secrets that were already tracked. Only `init` creates an overlay key. Restore a
missing identity from backup; `up` and `identity` never silently replace it.

Mining uses the shared Rust coordinator, with CPU workers by default. Add a
`mine` object to the host's `lattice.json`, for example:

```json
{"worker":"cpu","workers":2,"batchSize":100000,"recipients":{"Nexus":"ADDRESS"},"minWork":{"Nexus":"1"},"minBlockIntervalSeconds":2}
```

Replace `ADDRESS` with a valid public Lattice address. Unconfigured chains burn
rewards and fees. Mining only reads the node cookie and public recipient plan;
it never needs a wallet private key.

```sh
./target/debug/lattice --root "$HOME/.local/share/lattice" mine start
./target/debug/lattice --root "$HOME/.local/share/lattice" mine status
./target/debug/lattice --root "$HOME/.local/share/lattice" mine stop
./target/debug/lattice --root "$HOME/.local/share/lattice" mine run --once
```

`mine run` is foreground and responds to SIGINT/SIGTERM. `mine start` runs it in
the background with a private `log/mine.log`; duplicate starts fail. Stop/start
to apply edited mining settings. An executable `mine.worker` path selects the
shared process-worker protocol; relative paths resolve against the host root.
