# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0](https://github.com/ferrohd/tesmart/releases/tag/tesmart-cli-v0.1.0) - 2026-10-17

### Added

- *(cli)* print results to stdout and everything else to stderr
- *(cli)* define the json output contract separately from the library
- *(cli)* dispatch every command over one resolved target
- *(cli)* turn the global flags into a Kvm or a Matrix
- *(cli)* pin the exit code contract
- *(cli)* add the command surface and the global options

### Changed

- *(cli)* own target resolution, with every transport always built

### Documentation

- the library re-exports, the tool resolves target strings
