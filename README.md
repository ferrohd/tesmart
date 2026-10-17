# tesmart

Rust tooling for TESmart KVM switches and HDMI matrices, over TCP or RS232: a library, a command
line tool, and a transcription of the protocols they speak.

```sh
cargo install tesmart-cli                        # the `tesmart` binary
tesmart -t sim://HKS1601-EB23 -m HKS1601-EB23 switch 4   # try it without hardware
```

```toml
[dependencies]
tesmart = "0.1"
```

```rust
use tesmart::{Kvm, Port, model, tcp::TcpConnector};

# async fn example() -> Result<(), Box<dyn std::error::Error>> {
let info = model::lookup("HKS1601-EB23").expect("a model in the registry");
let mut kvm = Kvm::from_model("192.168.1.10".parse::<TcpConnector>()?, info)?;

let outcome = kvm.switch(Port::new(4)?).await?;
println!("{outcome}");
# Ok(())
# }
```

## What is here

| Crate | Role |
| --- | --- |
| [`tesmart`](crates/tesmart) | the front door: re-exports and feature-gated transports, behind one version |
| [`tesmart-proto`](crates/tesmart-proto) | the protocol, controller side: sans-IO, `no_std`, no allocator |
| [`tesmart-device`](crates/tesmart-device) | the protocol, switch side: answers like hardware, `no_std` |
| [`tesmart-client`](crates/tesmart-client) | the async driver and the operations |
| [`tesmart-tcp`](crates/tesmart-tcp) | one `Connector` over TCP |
| [`tesmart-serial`](crates/tesmart-serial) | one `Connector` over RS232 |
| [`tesmart-sim`](crates/tesmart-sim) | an emulated switch, for tests |
| [`tesmart-cli`](crates/tesmart-cli) | the `tesmart` binary, and target strings like `sim://HKS1601-EB23` |

## Three ideas

**Sans-IO at the core.** The protocol is a pure state machine: bytes in, events out, no sockets
and no clock. One crate awaits anything. That is what makes command spacing, retries and
resynchronisation after garbage testable without hardware, a network or a runtime — and what
lets the protocol run on a microcontroller. Both halves do: `tesmart-device` is the switch side,
so a bare-metal board can answer like a TESmart switch and existing software drives it unchanged.

**Say what was actually established.** This protocol acknowledges almost nothing. Of the
commands a KVM understands, only reading the active port and writing network settings produce a
reply, and several models cannot even do the former. So operations return outcomes rather than
`()`, and "the switch is on port 3" stays distinguishable from "port 3 was requested and nobody
can say what happened". Matrices are the exception: they report their routing table, so a route
is the one write in any family whose effect can be confirmed.

**Capabilities belong to the model, not the family.** Only "switch input" is documented for
every TESmart series; everything else depends on which PDF covers your unit, and several
commands are known to work only because community tools use them. The registry records that
difference instead of flattening it — `documented`, `field-tested`, `unverified` or
`unsupported` — so the tool can say *why* it is about to try something, or refuse.

## Protocols

Four incompatible families, and the same opcode byte means different things across them.
Every opcode, quirk and capability flag cites its source: an official TESmart document, or a
community tool observed to work.

See [`docs/protocol-reference.md`](docs/protocol-reference.md) for the transcription — including
the parts TESmart does not document, such as why the LAN port accepts configuration commands the
KVM firmware has never heard of — and [`docs/architecture.md`](docs/architecture.md) for how the
code is laid out.

## Development

```sh
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features
cargo build -p tesmart-proto  --target thumbv7em-none-eabi   # controller side, bare metal
cargo build -p tesmart-device --target thumbv7em-none-eabi   # switch side, bare metal
```

A Nix flake provides the toolchain: `nix develop`.

## Licence

AGPL-3.0-or-later. See [LICENSE](LICENSE).

For commercial licensing options (use without AGPL obligations), contact the project maintainer.

## Disclamer

This project is not affiliated with [TESmart](https://tesmart.com/). It is an independent, community-developed library.
