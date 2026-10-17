# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0](https://github.com/ferrohd/tesmart/releases/tag/tesmart-proto-v0.1.0) - 2026-10-17

### Added

- *(proto)* classify a decoded chunk without borrowing the decoder
- *(proto)* tell an unsolicited report from a pending answer
- *(proto)* expose the event queue and count what it dropped
- *(proto)* retry a query on a missed deadline and fail loudly after
- *(proto)* withhold the bytes until the command gap has passed
- *(proto)* refuse a request outside the profile before a byte is sent
- *(proto)* add the Engine and its policy accessors
- *(proto)* model a pending exchange and what it will accept
- *(proto)* add the engine event and error vocabulary
- *(proto)* collect the field-tested timings into a Policy
- *(proto)* identify an exchange so a reply can be matched to it
- *(proto)* register the 8x8, 16x16 and sdi models
- *(proto)* register the 4x4 matrices, with and without a LAN port
- *(proto)* register the dual-monitor switch
- *(proto)* register the Family A models with no published protocol
- *(proto)* register the EB23 and P23 series and their field reports
- *(proto)* register the single-monitor Family A models
- *(proto)* name the capability set of every protocol document
- *(proto)* add ModelInfo and let the family have the last word
- *(proto)* add Capabilities with the provenance of each claim
- *(proto)* grade a capability by how well established it is
- *(proto)* dispatch over the families at run time
- *(proto)* add the sdi switch, four bytes and no terminator
- *(proto)* add the matrix table as the union of all four command sets
- *(proto)* add the dual-monitor table and its inverted pass-through
- *(proto)* add the single-monitor KVM command table
- *(proto)* share the two opcodes every hex family agrees on
- *(proto)* explain what a clashing opcode would have done instead
- *(proto)* add the four protocol families with no default
- *(proto)* seal the Profile trait so families stay exhaustive
- *(proto)* add CommandInfo so a table can describe itself
- *(proto)* add ResponseKind for matching an in-flight exchange
- *(proto)* add Response and the kind an exchange is matched against
- *(proto)* add RequestKind as the key profiles are written against
- *(proto)* name and classify every request
- *(proto)* add the family-independent Request type
- *(proto)* resync past garbage without losing the frame behind it
- *(proto)* scan for a frame, text, or garbage to discard
- *(proto)* buffer a byte stream and hand out one chunk at a time
- *(proto)* add the decoder chunk vocabulary and the two directions
- *(proto)* dispatch a text command between the two languages
- *(proto)* parse the zero-padded network replies
- *(proto)* encode network queries and writes in both dialects
- *(proto)* add the two ascii dialects and the two scopes
- *(proto)* print link ports as 1-9 then A-G
- *(proto)* add the LINK routing report in both directions
- *(proto)* parse the MT00 matrix commands and the RD trap
- *(proto)* add the MT00 matrix command vocabulary
- *(proto)* encode hex frames with a derived length byte
- *(proto)* document the frame header, terminator and length byte
- *(proto)* add the text helpers both ascii languages need
- *(proto)* add Transmit so the engine can label and retry bytes
- *(proto)* build commands with write! and no allocator
- *(proto)* print protocol bytes the way the documents do
- *(proto)* add a copyable byte buffer sized for the 16x16 link reply
- *(proto)* add the four network settings and a partial config
- *(proto)* add the matrix display timeout and preset slots
- *(proto)* bound the auto-switch interval at five seconds
- *(proto)* add relative Step and the hotkey trigger
- *(proto)* add Toggle and the dual-monitor output
- *(proto)* add the buzzer and display timeout settings
- *(proto)* add a deterministic Tick clock for tests
- *(proto)* add a Timestamp trait so the engine never reads a clock
- *(proto)* add a drop-oldest ring for the engine event queue
- *(proto)* add a fixed-capacity byte fifo for the stream decoder
- *(proto)* gate ports on a per-model PortCount
- *(proto)* add Port with the write/read byte asymmetry

### Documentation

- *(proto)* document the engine loop and what it protects you from
