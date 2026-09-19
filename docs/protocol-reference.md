# TESmart KVM Control Protocol Reference

Oct 15, 2026 · @Alessandro · revised Oct 16, 2026

## Scope: four protocol families

TESmart uses four incompatible control protocols, and the same opcode byte means different things across them. Identify your family before sending anything.

| Family | Devices | Wire format | Example |
| --- | --- | --- | --- |
| A. Hex KVM | Single-monitor 4/8/16-port KVMs with RS232/LAN (HKS0801A\*, HKS1601A\*, HSW0801A\*, HSW1601A\*, HKE0802A10, DKS0401A30, HKS801-EB23, HKS1601-EB23, HKS801-P23, HKS1601-P23) | 6-byte binary frames | `AA BB 03 01 02 EE` |
| A'. Hex KVM, dual-monitor | 4-port dual-monitor KVMs (community-documented) | Same framing, different opcodes from 0x03 up | `AA BB 03 03 01 EE` = "Output A input +1" |
| B. Matrix ASCII | HDMI matrices (HMA0404\*, HME04\*, HMA0808\*, HMA1616\*) | ASCII strings | `MT00SW0102NT` |
| C. SDI switch | The 4x1 SDI switch (SSW04010A10) | 4-byte binary, **no terminator** | `AA CC 01 02` |

Family A also carries an ASCII side-channel for network settings (`IP:`, `PT:`, `GW:`, `MA:`), covered below. Models not listed here (for example HKS801-M24 / HKS401-M24, DKS203-M24, HKS801-L23, PKS0402A20, 8K60 HDMI 2.1) have no published protocol I could find; treat them as unverified until tested.

Rule of thumb for a CLI: make the family an explicit setting (`--family kvm|kvm-dual|matrix|sdi`), never auto-guess from the model name.
