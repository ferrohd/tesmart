# tesmart-rs

Rust tooling for TESmart KVM switches and HDMI matrices, over TCP or RS232: a library, a command
line tool, and a transcription of the protocols they speak.

## Protocols

Four incompatible families, and the same opcode byte means different things across them.
Every opcode, quirk and capability flag cites its source: an official TESmart document, or a
community tool observed to work.

See [`docs/protocol-reference.md`](docs/protocol-reference.md) for the transcription — including
the parts TESmart does not document, such as why the LAN port accepts configuration commands the
KVM firmware has never heard of — and [`docs/architecture.md`](docs/architecture.md) for how the
code is laid out.

## Licence

AGPL-3.0-or-later. See [LICENSE](LICENSE).
