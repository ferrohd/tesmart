# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0](https://github.com/ferrohd/tesmart/releases/tag/tesmart-client-v0.1.0) - 2026-10-17

### Added

- *(client)* report a change once, whichever way it was noticed
- *(client)* hold the connection open for as long as a watch lives
- *(client)* follow the active port by both report and poll
- *(client)* build a matrix without pretending it is a KVM
- *(client)* read the routing back and check it against what was asked
- *(client)* load and save presets, gated on the matrix series
- *(client)* route, mirror and confirm it from the reported table
- *(client)* add the Matrix handle with inputs and outputs apart
- *(client)* add the route options and the matrix verify switch
- *(client)* refuse to guess a family or a port count
- *(client)* probe how many inputs a switch really has
- *(client)* read and write network settings over the side-channel
- *(client)* add the write-only settings and the dual-monitor commands
- *(client)* read the port back after a switch, loudly if it disagrees
- *(client)* add the Kvm handle over a borrowed session
- *(client)* add the switch options and the three verify modes
- *(client)* resolve family and port count the same way for both builders
- *(client)* refuse a cross-family model and say which it was
- *(client)* share the network side-channel between both families
- *(client)* reconnect per command and clear a stale exchange
- *(client)* log every event and queue the unsolicited ones
- *(client)* select on the read, the engine deadline and the budget
- *(client)* turn the engine's poll interface into async functions
- *(client)* put the engine in the session, not in the connection
- *(client)* count what happened on the wire
- *(client)* add the warnings that are true but are not errors
- *(client)* confirm a matrix route from the reported routing table
- *(client)* report which network values answered and which read back
- *(client)* report a write-only command as sent, and a probed count
- *(client)* say whether a switch was verified, assumed or already set
- *(client)* describe the attached switch, catalogued or not
- *(client)* fold the broken-pipe family into a closed link
- *(client)* let the transport decide the ascii scope
- *(client)* choose a connection per command or a persistent one
- *(client)* define the Connector transport boundary
- *(client)* classify a failure so a caller can map it to an exit code
- *(client)* add OpError carrying what was attempted
- *(client)* separate a failed exchange from a failed operation

### Documentation

- *(client)* tabulate what each operation can and cannot promise
- *(client)* document the loop order and the three tracing targets
