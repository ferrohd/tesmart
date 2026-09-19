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

### Official model-to-document mapping

From TESmart's own index, "LAN RS232 Control Protocols", updated 2024-01-10. This is the authoritative list; the attachment IDs are stable and link directly to each PDF.

| Document | Models |
| --- | --- |
| [4x1 SDI switch](https://support.tesmart.com/hc/en-us/article_attachments/10269851185433) | SSW04010A10 (the PDF itself says SSW0401A10) |
| [4x1 DVI KVM](https://support.tesmart.com/hc/en-us/article_attachments/10269851170969) | DKS0401A30 |
| [8x1 HDMI KVM](https://support.tesmart.com/hc/en-us/article_attachments/10269851472793) | HKS0801A20 |
| [8x1 HDMI Switch (KVM) v2](https://support.tesmart.com/hc/en-us/article_attachments/10269851509913) | HKS0801A30, HSW0801A10, HSW0801A1U, HKE0802A10, HKS0801A40, HKS0801A1U |
| [16x1 HDMI KVM](https://support.tesmart.com/hc/en-us/article_attachments/10269851662361) | HKS1601A10, HSW1601A10, HKS1601A1U, HSW1601A1U |
| [4x4 HDMI matrix (RS232)](https://support.tesmart.com/hc/en-us/article_attachments/10269851354137) | HMA0404A40, HMA0404B40, HMA0404B50, HMA0404A50, HMA0404A30, HMA0404A70 |
| [4x4 HDMI matrix (RS232+IP)](https://support.tesmart.com/hc/en-us/article_attachments/10269851379737) | HMA0404B30, HME040410H, HME040806R, HMA0404A1U |
| [8x8 HDMI matrix](https://support.tesmart.com/hc/en-us/article_attachments/10269851646617) | HMA0808A10, HMA0808A20, HMA0808A30, HMA0808A1U |
| [16x16 HDMI matrix](https://support.tesmart.com/hc/en-us/article_attachments/27717852679449) | HMA1616A10, HMA1616A20 |

Note that the EB23 and P23 series appear in **no** protocol document; their commands come from the user manuals, and the rest from field reports.

> **Reading the PDFs.** They are text-based but awkward to extract. `https://r.jina.ai/<pdf-url>` renders any of them to plain text, which is how every table here was transcribed and re-verified.
