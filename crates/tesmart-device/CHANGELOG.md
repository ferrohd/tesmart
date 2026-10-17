# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0](https://github.com/ferrohd/tesmart/compare/tesmart-device-v0.1.1...tesmart-device-v0.2.0) - 2026-10-17

### Changed

- [**breaking**] name the protocol once, and drop the A/B/C letters

## [0.1.0](https://github.com/ferrohd/tesmart/releases/tag/tesmart-device-v0.1.0) - 2026-10-17

### Added

- *(device)* handle the Family A and A' frames in one place
- *(device)* handle the matrix commands the model admits to having
- *(device)* route a matrix input to an output and report it back
- *(device)* answer network queries padded and split across segments
- *(device)* honour a command only if the model supports it
- *(device)* add the emulated switch and its front panel
- *(device)* hold the switch state with the factory defaults
- *(device)* configure an emulated switch from a registry model
- *(device)* add deliberate faults to reproduce what hardware does

### Documentation

- *(device)* document both of its uses and the module layout
