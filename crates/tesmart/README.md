# tesmart

Control TESmart KVM switches and HDMI matrices over TCP or RS232.

```toml
[dependencies]
tesmart = "0.1"
```

```rust
use tesmart::Port;

# async fn example() -> Result<(), Box<dyn std::error::Error>> {
let mut kvm = tesmart::kvm("192.168.1.10", "HKS1601-EB23")?;

match kvm.switch(Port::new(4)?).await? {
    outcome if outcome.is_confirmed() => println!("on port {}", outcome.port()),
    outcome => println!("sent, but unconfirmable: {outcome}"),
}
# Ok(())
# }
```

Nothing connects until you call an operation, so building a handle cannot fail for network
reasons.

## Why the outcome types

This protocol acknowledges almost nothing. Of the commands a KVM understands, only reading the
active port and writing network settings produce a reply, and several models cannot even do the
former. So operations return outcomes rather than `()`, and "the switch is on port 3" stays
distinguishable from "port 3 was requested and nobody can say what happened".

Matrices are the exception: they report their whole routing table, so `Matrix::route` is the one
write in any TESmart family whose effect can actually be confirmed.

The crate also refuses commands that do not exist on the attached model, before any byte is
sent, and says why:

```text
step-output is not available on the kvm profile: on single-monitor switches opcode 0x03
sets the front display timeout; use switch to choose a port
```

That mistake is worth refusing: `0x03` sets a display timeout on one family and changes which
computer you are looking at on another.

## Features

| Feature | Default | Effect |
| --- | --- | --- |
| `tcp` | yes | the switch's LAN port (192.168.1.10:5000 by default) |
| `serial` | no | the switch's RS232 port, 9600 8N1 |
| `sim` | no | an emulated switch, for tests and for trying things without hardware |
| `serde` | no | `Serialize`/`Deserialize` on the protocol value types |

A target string naming a transport you did not enable is an error, not a fallback.

```rust
// tesmart = { version = "0.1", features = ["serial", "sim"] }
# fn example() -> Result<(), Box<dyn std::error::Error>> {
let lan    = tesmart::resolve("192.168.1.10")?;      // tcp
let serial = tesmart::resolve("/dev/ttyUSB0@9600")?; // serial
let fake   = tesmart::resolve("sim://HKS1601-EB23")?;// emulated
# Ok(())
# }
```

## Testing without hardware

With the `sim` feature, `tesmart-sim` emulates a switch — including the parts that only show up
on real units: replies split across segments, terminators that are not `0xEE`, missing replies,
and port reports nobody asked for. Faults are counter-driven rather than random, so a failing
test fails the same way twice.

## Layers

| Module | Crate | Role |
| --- | --- | --- |
| `proto` | [`tesmart-proto`] | the protocol, controller side: sans-IO, `no_std`, no allocator |
| `client` | [`tesmart-client`] | the async driver and the operations |
| `tcp` | [`tesmart-tcp`] | one `Connector` over TCP |
| `serial` | [`tesmart-serial`] | one `Connector` over RS232 |
| `sim` | [`tesmart-sim`] | serves a [`tesmart-device`] behind a `Connector` |

Depend on `tesmart-proto` directly if you want the protocol on a microcontroller: it is
`no_std`, has no allocator and no dependency beyond `thiserror`, and is built for
`thumbv7em-none-eabi` in CI.

[`tesmart-device`] is the other half — the switch side, also `no_std` — so a bare-metal board
can answer like a TESmart switch and software written for the real hardware drives it unchanged.

## Command line tool

The `tesmart` binary ships in a separate crate, so that a library consumer does not pull in an
argument parser:

```sh
cargo install tesmart-cli
```

## Protocol documentation

Every opcode, quirk and capability flag in this project cites its source — either an official
TESmart document or a community tool observed to work. See
[`docs/protocol-reference.md`](https://github.com/ferrohd/tesmart/blob/master/docs/protocol-reference.md).

## Licence

AGPL-3.0-or-later.

[`tesmart-proto`]: https://crates.io/crates/tesmart-proto
[`tesmart-device`]: https://crates.io/crates/tesmart-device
[`tesmart-client`]: https://crates.io/crates/tesmart-client
[`tesmart-tcp`]: https://crates.io/crates/tesmart-tcp
[`tesmart-serial`]: https://crates.io/crates/tesmart-serial
[`tesmart-sim`]: https://crates.io/crates/tesmart-sim
