# tesmart-cli

Command line control of TESmart KVM switches and HDMI matrices, over TCP or RS232.

```sh
cargo install tesmart-cli      # installs a binary called `tesmart`
```

The crate is `tesmart-cli`; the binary is `tesmart`. The library lives in the
[`tesmart`](https://crates.io/crates/tesmart) crate.

## Usage

```sh
# every switch leaves the factory on 192.168.1.10:5000, which is the default target
tesmart -m HKS1601-EB23 switch 4
tesmart -m HKS1601-EB23 get

# over RS232
tesmart -t /dev/ttyUSB0 -m HKS801-P23 switch 2

# try it with no hardware at all
tesmart -t sim://HKS1601-EB23 -m HKS1601-EB23 switch 4

# follow the active port, however it changes: front panel, hotkey, IR remote
tesmart -m HKS1601-EB23 watch

# HDMI matrices are a different protocol, and get their own subcommands
tesmart -t 192.168.1.10 -m HMA0808A10 matrix route 3 2
tesmart -t 192.168.1.10 -m HMA0808A10 matrix show
```

## Honest output

The protocol acknowledges almost nothing, so the tool distinguishes what it confirmed from what
it merely sent:

```text
$ tesmart -m HKS1601-EB23 switch 4
switched to port 4

$ tesmart -m DKS0401A30 switch 3
port 3 requested, not verified: reading the active port is unsupported on this model

$ tesmart -m HKS1601-EB23 buzzer off
buzzer sent, state cannot be read back
```

`--require-verified` turns the middle case into a failure. `--output json` carries the same
distinction, and the exit code says which:

| Code | Meaning |
| --- | --- |
| 0 | success, including a command that was sent but cannot be verified |
| 2 | usage: bad arguments, no model, a port this switch does not have |
| 3 | transport: cannot connect, or the link died |
| 4 | timeout: nothing came back in time |
| 5 | unsupported: this switch cannot do that |
| 6 | verification: the switch reports something else |
| 7 | protocol: the switch said something that makes no sense |

## Which model?

`-m` is required for anything that touches a switch, and is never guessed — the same opcode
means different things across TESmart's four protocols, and `0x03` either sets a display
timeout or changes which computer you are looking at.

```sh
tesmart models list              # everything the registry knows
tesmart models show HKS801-P23   # per-command support, and the evidence for it
```

For a switch that is not in the registry, give `-p kvm --ports 8` instead. `tesmart probe` will
try the capabilities the registry is unsure about and report what actually worked.

## Licence

AGPL-3.0-or-later.
