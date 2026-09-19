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

## Family A: hex KVM protocol

Every command is 6 raw bytes, identical over RS232 and TCP; there is no checksum and no ACK beyond the read-port reply.

| Byte | Value | Meaning |
| --- | --- | --- |
| 0 | `0xAA` | Header |
| 1 | `0xBB` | Header |
| 2 | `0x03` | Length: payload + 2. Confirmed by the dual-monitor `0x04` frame, which carries two data bytes |
| 3 | opcode | Command |
| 4 | value | Argument (ignored on reads) |
| 5 | `0xEE` | Terminator |

**Transport defaults (all Family A docs agree)**

| Setting | RS232 | LAN |
| --- | --- | --- |
| Link | 9600 baud, 8 data bits, 1 stop bit, **parity none**, no flow control | TCP, raw stream (no Telnet negotiation) |
| Address | 3-pin Phoenix connector (DB9 adapter needed) | 192.168.1.10, port 5000 |
| Gateway / mask | n/a | 192.168.1.1 / 255.255.255.0 |
| DHCP | n/a | Not supported |

Parity and flow control are stated explicitly in the P23 and EB23 manuals and in all three matrix protocol documents, so 8N1 is documented rather than inferred.

**Port numbering is asymmetric.** Writes are 1-based (`0x01` = PC1 … `0x10` = PC16). The read reply is 0-based (`0x00` = PC1 … `0x0F` = PC16). PiKVM's driver encodes exactly this, adding 1 on write and none on read.

**Read reply.** Send `AA BB 03 10 00 EE`; the switch answers `AA BB 03 11 XX EE` with `XX` = active port, 0-based.

**Out-of-range writes are ignored.** A switch command naming a port the unit does not have changes nothing and is not acknowledged. That is a usable probe: switch to 16, 15, 14 … and read back; the first candidate that reads back as itself is the real input count. `krohrbaugh/teeheesmart` detects port counts this way.

## Family A: commands by series

Only "switch input" is documented for every series; everything else depends on which TESmart PDF covers your model. "Doc" = in the official PDF or manual; "Field" = absent from that doc but sent by working community tools; blank = no evidence.

| Opcode | Value | Function | 4-port DVI DKS0401A30 | 8x1 v1 HKS0801A20 | 8x1 v2 HKS0801A30/A40/A1U, HSW0801A10/A1U, HKE0802A10 | 16x1 HKS1601A10/A1U, HSW1601A10/A1U | EB23 HKS801/1601-EB23 | P23 HKS801/1601-P23 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `0x01` | `0x01`–`0x10` | Switch to PC n (1-based) | Doc (1–4) | Doc (1–8) | Doc (1–8) | Doc (1–16) | Doc (1–16) | Doc (1–16) |
| `0x02` | `0x00` / `0x01` | Buzzer mute / unmute |  |  | Doc | Doc | Doc | Doc |
| `0x03` | `0x0A` / `0x1E` / `0x00` | Front display timeout 10 s / 30 s / never |  |  | Doc | Doc | Doc | Doc |
| `0x81` | `0x00` / `0x01` | Auto input detect off / on |  |  | Doc | Field | Doc | Doc |
| `0x10` | `0x00` | Read active port |  | Doc | Doc | Doc | Field | — |
| `0x11` | port (0-based) | Reply: active port (switch to host) |  | Doc | Doc | Doc | Field | — |

The display-timeout values are plain decimal seconds: `0x0A` = 10, `0x1E` = 30. That is worth knowing because it suggests the field is a seconds count rather than an enum, though no document confirms other values work.

Notes per column:

- **DKS0401A30**: the PDF lists only the four switch commands; no read, no settings.
- **EB23 and P23 manuals** list opcodes 0x01, 0x03, 0x02 and 0x81 only — verified by reading both manuals. Neither documents `0x10`/`0x11`. Read-port works on EB23 in practice ([luborpetr/tesmart](https://github.com/luborpetr/tesmart) and the [tesmart\_kvm HA integration](https://github.com/tobydoescode/hass-integration-tesmart-kvm) poll it). For P23 there is still no field report: test before relying on it.
- **EB23 vs P23**: EB23 is 4K30 with USB 2.0, P23 is 4K60 with USB 3.2 Gen1. The protocol content of the two manuals is identical. (Their hotkey tables disagree about which of PgUp/PgDn means "next", which is a documentation error, not a protocol difference.)
- **Write-only settings**: buzzer, display timeout and auto-detect cannot be read back on any series; only the active port and network settings can.
- **Hotkeys and other out-of-band changes** (not protocol commands, but they matter): the port also changes via the front panel, the IR remote, hotkeys (Right-Ctrl twice then a digit; Scroll Lock after Right-Ctrl, Right-Ctrl, F1), and a double-click of the mouse wheel. Anything tracking the active port must expect it to move without having asked, which is what makes the unsolicited reports below worth listening for.

## Family A': dual-monitor 4-port variant

This variant reuses the Family A framing but redefines opcodes 0x03 and up. Sending a Family A "display timeout" (`0x03`) here steps output A's input instead. Source: [spuder/Tesmart-KVM](https://github.com/spuder/Tesmart-KVM), tested on TESmart's 4-port dual-monitor HDMI/USB-C 4K60 KVM over RS232 at 9600 baud. No official TESmart PDF covers it.

| Frame | Function | Clash with Family A |
| --- | --- | --- |
| `AA BB 03 01 0n EE` | Switch to PC n (1–4) | Same |
| `AA BB 03 02 00 EE` / `… 02 01 EE` | Buzzer off / on | Same |
| `AA BB 03 03 00 EE` / `… 03 01 EE` | Output A input −1 / +1 | **Yes**: 0x03 = display timeout in A |
| `AA BB 03 04 00 EE` / `… 04 01 EE` | Output B input −1 / +1 | New |
| `AA BB 03 05 01 EE` | Move USB between outputs (display mode 2) | New |
| `AA BB 03 06 00 EE` / `… 06 01 EE` | K/M pass-through on / off (note inverted values) | New |
| `AA BB 03 07 01 EE` / `… 07 00 EE` | Hotkey trigger Right-Ctrl / Scroll Lock | New |
| `AA BB 03 08 01 EE` / `… 08 00 EE` | Auto-switch on / off | New |
| `AA BB 04 09 MM SS EE` | Auto-switch interval, 0x00–0x38, minimum 5 s | New; 7 bytes, byte 2 = `0x04` |

TESmart support told the author there is no exact equivalent of the Output A/B front buttons; the ±1 commands are the closest. No read command is documented for this variant.

The 7-byte frame with `0x04` in byte 2 is why byte 2 is a length field. A parser should not hard-code 6-byte frames for this family.

**`MM SS` is minutes and seconds.** The source lists the two bytes without naming them. The P23 and EB23 manuals describe the same auto-scan feature on the front panel as separate `[MIN]` and `[SEC]` settings, 0–59 each, minimum 5 s, default 15 s, "5 seconds to 1 hour" — which makes minutes-then-seconds the only reading consistent with the hardware. One discrepancy remains: the serial command's documented ceiling is `0x38` (56) per field, while the panel allows 59.

## Family A: ASCII network-config commands

On Family A KVMs, IP settings use plain ASCII on the same TCP socket, with no `AA BB` framing. None of this is in TESmart's protocol PDFs; it comes from packet captures of the official Windows controller ([bbeaudoin](https://github.com/bbeaudoin/bash/blob/master/tesmart/README.md)) and working tools ([pschmitt/tesmart.sh](https://github.com/pschmitt/tesmart.sh), [luborpetr/tesmart](https://github.com/luborpetr/tesmart), [SiirRandall/tesmart-ui](https://github.com/SiirRandall/tesmart-ui)).

| Purpose | Send | Reply |
| --- | --- | --- |
| Query IP | `IP?` | `IP:010.000.002.012` then `;` (may arrive as two TCP segments) |
| Query port | `PT?` | `PT:05000;` |
| Query gateway | `GW?` | `GW:010.000.002.001` then `;` |
| Query mask | `MA?` | `MA:255.255.255.000` then `;` |
| Set IP | `IP:10.0.2.12;` | `OK` |
| Set port | `PT:5000;` | `OK` |
| Set gateway | `GW:10.0.2.1;` | `OK` |
| Set mask | `MA:255.255.255.0;` | `OK` |

Spelling differences between sources, both reported working:

- Captured controller traffic: `IP:10.0.2.12;` (no space) and `IP?` (no semicolon). This is the canonical form: it is also what the matrix documents print.
- tesmart.sh: `IP: 10.0.2.12;` (space after colon) and `IP?;` (with semicolon).
- Replies are zero-padded (`010.000.002.012`, `05000`); strip leading zeros per octet when parsing.

Applying changes:

1. Settings persist on `OK` but take effect only after a power cycle.
2. Verify with the four queries before power-cycling. There is no factory reset; a wrong IP leaves RS232 as the only way back.
3. RS232 support for these commands is disputed on KVMs — see the next section for why.

### Why the LAN port has commands the KVM does not

The ASCII network channel is not a KVM command set at all. bbeaudoin's board photographs identify the LAN port as a **CH9120/CH9121-class Ethernet-to-UART bridge**: a separate chip that terminates TCP and forwards a byte stream to the KVM's MCU over serial. `IP?`, `PT:`, `GW:` and `MA:` are that bridge's own configuration protocol, intercepted before they ever reach the KVM.

That explains every oddity about them at once:

- they are absent from every TESmart *protocol* document, because they are not the KVM's commands;
- TESmart's IP-change guide can truthfully say the KVM firmware "does not support IP configuration via RS232" — over RS232 you are talking to the MCU, and the bridge is not in the path;
- they share a socket with the hex frames without any framing or collision, because the bridge simply passes through anything it does not recognise;
- the ~48-byte segmentation and the lone `;` are the bridge's MTU, not a protocol feature.

Community reports of configuring IP over serial ([bbeaudoin](https://github.com/bbeaudoin/bash/blob/master/tesmart/README.md), [luborpetr](https://github.com/luborpetr/tesmart)) are not necessarily wrong — some units may wire the bridge's config UART differently — but the mechanism says to treat RS232 recovery as untested on your unit.

Matrices are the exception: TESmart documents their network commands for **both** transports, in two different spellings. See below.

## Family B: matrix ASCII protocol (not for KVMs)

TESmart HDMI matrices use ASCII command strings on the same default address (192.168.1.10:5000, 9600 8N1), so a KVM tool pointed at a matrix will connect but do nothing useful. Sources: the 4x4 (RS232), 4x4 (RS232+IP), 8x8 and 16x16 protocol documents, all four transcribed.

| Command | Meaning |
| --- | --- |
| `MT00SWXXYYNT` | Route input XX to output YY |
| `MT00SWXX00NT` | Input XX to all outputs |
| `MT00SW0000NT` | Mirror 1→1, 2→2 … |
| `MT00RD0000NT` | Request routing; reply `LINK:OYIX;END` |
| `MT00RD01XXNT` | Load routing preset XX (`01`–`08`) |
| `MT00SV00XXNT` | Save current routing into preset XX (`01`–`08`) |
| `MT00BZEN01NT` / `MT00BZEN00NT` | Buzzer **mute** / unmute |
| `MT00CTXXXXNT` | Display timeout, 0000–9999 s (0000 = never) |

### One family, four command sets

The table above is the union. No matrix implements all of it, and the gaps do not nest:

| Models | Route, mirror, read | `MT00CT` | Buzzer | Presets | Network |
| --- | --- | --- | --- | --- | --- |
| 4x4, RS232 only (A40, B40, B50, A50, A30, A70) | yes | no | no | no | **no** |
| 4x4, RS232+IP (B30, HME040410H, HME040806R, A1U) | yes | **yes** | no | no | yes |
| 8x8 (A10, A20, A30, A1U) | yes | no | **yes** | **yes** | yes |
| 16x16 (A10, A20) | yes | no | **yes** | **yes** | yes |

So `MT00CT` and the preset/buzzer commands are mutually exclusive in practice. The RS232-only 4x4 document is four commands long: routing and the `LINK:` report, with no network section at all.

### Two traps

**The buzzer is inverted.** On a matrix, `MT00BZEN01NT` *mutes*. On a KVM, `0x02` with `0x00` mutes. Both are printed that way in TESmart's own documents. Any code sharing a "buzzer on/off" value between the families must invert it in exactly one place.

**`MT00RD` is two commands.** `MT00RD0000NT` reads the routing; `MT00RD01XXNT` loads preset XX. They differ by one character, and the second one changes what is on screen.

### The `LINK:` reply

Documented as `LINK:OYIX;END`, where **Y is the output and X is the input** — the `O` and `I` prefixes name the following digit. How a multi-output matrix lays out the rest is not stated, but the pairs are semicolon-separated in every example.

The port digits are **not plain decimal**:

| Models | Digits |
| --- | --- |
| 4x4 | `1`–`4` |
| 8x8 | `1`–`8` |
| 16x16 | `1`–`9`, then `A`, `B`, `C`–`G` |

So on a 16x16, output 10 is `A` and output 16 is `G`. A decimal parser silently rejects every reply above port 9.

This also sets the largest reply in the whole protocol. A full 16x16 report is `LINK:` + 16 × `OnIm;` + `END` = **88 bytes**, which does not fit the ~48-byte bridge segment and therefore always arrives split. A decoder's reassembly buffer has to be sized for the logical unit, not for the MTU.

### Network commands, per transport

Every matrix document lists these twice, once marked "for LAN connection only" and once "for RS232 connection only".

| Purpose | LAN | RS232 |
| --- | --- | --- |
| Query IP / port / gateway / mask | `IP?` `PT?` `GW?` `MA?` | `MT8003IP?` `MT8003PT?` `MT8003GW?` `MT8003MA?` |
| Set IP | `IP:AAA.BBB.CCC.DDD;` | `MT8019IP:AAA.BBB.CCC.DDD;` |
| Set port | `PT:XXXXX;` | **`MT8009PT:XXXXX;`** |
| Set gateway | `GW:AAA.BBB.CCC.DDD;` | `MT8019GW:AAA.BBB.CCC.DDD;` |
| Set mask | `MA:AAA.BBB.CCC.DDD;` | `MT8019MA:AAA.BBB.CCC.DDD;` |
| Reply (either transport) | `IP:AAA.BBB.CCC.DDD;` and friends | identical |

Three things to note. Queries all share the `MT8003` prefix, but the writes do not: the port write is `MT8009` where the other three are `MT8019`. That inconsistency is printed identically in all three documents that have a network section, so it is transcribed rather than corrected. The replies are the same bare strings in both scopes, so only the encoder needs to know which transport it is on. And the documents write the values zero-padded to fixed width (`AAA.BBB.CCC.DDD`, `XXXXX`), unlike the captured LAN traffic on KVMs, which is unpadded.

Power-cycling applies here too: "Turn off matrix and on again, and wait about 2 seconds", and "please do remember the modified IP settings" — there is no reset.

## Family C: the SDI switch

One model, one command. Source: [4x1 SDI switch protocol](https://support.tesmart.com/hc/en-us/article_attachments/10269851185433), 9600 8N1, parity none.

| Command | Bytes |
| --- | --- |
| Switch to input 1 | `AA CC 01 01` |
| Switch to input 2 | `AA CC 01 02` |
| Switch to input 3 | `AA CC 01 03` |
| Switch to input 4 | `AA CC 01 04` |

Four bytes; header `AA CC` rather than `AA BB`; **no terminator** — nothing corresponds to the `0xEE` that ends a KVM frame. Whether the `01` is a length byte, as it is in `AA BB 03 …`, is not stated; since the document lists the four commands literally and defines nothing else, it is safest to treat the first three bytes as a fixed prefix.

The switch reports nothing: no read command, no acknowledgement. Every command in this family is unverifiable by construction.

## Transport behaviour and quirks

The LAN port is a serial-to-Ethernet bridge (see above), so expect serial-like behaviour over TCP: unsolicited bytes, split packets and occasional garbage. Observations from field tools, none officially documented:

| Quirk | Detail | Source |
| --- | --- | --- |
| Unsolicited port reports | While a client is connected, the switch pushes `AA BB 03 11 XX …` when the port changes by any means (button, hotkey, IR, mouse wheel). Stops after the client closes cleanly. | bbeaudoin |
| Bad terminator | Replies ending in `0x16` or `0x1C` instead of `0xEE` have been seen. Match on `AA BB 03 11` + 1 data byte + any 1 byte. | bbeaudoin |
| Invalid read data | Read replies are sometimes wrong, truncated or missing; retry. tesmart.sh retries up to 10 times, 0.2 s apart. | bbeaudoin, tesmart.sh |
| `0xFF` on invalid input | Garbage in gets `0xFF` back on some units. | bbeaudoin |
| Rate limit | Space commands at least 1 s apart. PiKVM waits 1.0 s after each switch. | bbeaudoin, PiKVM |
| Small MTU | About 48 bytes per segment; ASCII replies split (the `;` arrives alone). Read until `;`, `END` or `OK`. | bbeaudoin |
| One connection per command | PiKVM opens, sends, reads exactly 6 bytes, and closes for every command; 5 s timeouts; polls every 10 s. | PiKVM kvmd |
| Old 10 Mbps NIC | Newer units shipped a 10 Mbps-only controller that may not link with 2.5/5/10 GbE ports; use a 1 GbE switch port. | bbeaudoin |
| No authentication | Anyone who can reach port 5000 can switch. Keep it off the internet. | All |

### The `0x11 → PC1` mystery, solved

`tesmart.sh` maps a read value of 17 (`0x11`) to PC1, which corresponds to no documented behaviour. It is a **bug in the script's own parsing**, not a protocol quirk.

The script extracts the port with `hexdump -C | awk '{print $(NF-2)}'`, taking the third-from-last field of the line. On a complete 6-byte reply (`AA BB 03 11 XX EE`) that lands on `XX`, the port. On a **truncated** 5-byte reply — which this hardware does produce — the fields shift and `$(NF-2)` lands on the opcode `0x11` = 17 instead. The `17 → PC1` mapping is a workaround for the script's own off-by-one field, and it is wrong in general: it silently reports PC1 whenever a reply arrives short.

The lesson is for the decoder, not the mapping table: match frames by structure (header, length, opcode) and treat a short frame as incomplete rather than parsing whatever bytes arrived.

`SiirRandall/tesmart-ui` has the mirror-image bug: its frame scanner requires the terminator to be exactly `0xEE`, so it drops every reply that ends in `0x16` or `0x1C`.
