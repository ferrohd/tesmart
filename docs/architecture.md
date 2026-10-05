# tesmart-rs: Architecture

How the workspace is layered, and why. For the wire-level details (bytes, opcodes, per-model support), see the companion [TESmart KVM Control Protocol Reference](protocol-reference.md).

## Principles

**Sans-IO at the core.** The protocol is a pure state machine: bytes in, events out, no sockets and no clock. Everything that awaits lives in exactly one crate. This is what makes the hard parts — command spacing, retries, resynchronisation after garbage — testable without hardware, a network, or a running runtime.

**Almost everything is shared.** A protocol family is only an opcode table (data, not code), a codec choice, and a few framing quirks.

**Capabilities belong to the model, not the family.** The family says which commands *exist*; the registry says which ones are believed to *work*, and on what evidence. Within the matrix family alone there are four different command sets.

**Say what was actually established.** The protocol acknowledges almost nothing, so operations return outcomes rather than `()`. "The switch is on port 3" and "port 3 was requested and nobody can confirm it" must stay distinguishable all the way to the user.
