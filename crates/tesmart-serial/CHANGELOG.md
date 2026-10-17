# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0](https://github.com/ferrohd/tesmart/releases/tag/tesmart-serial-v0.1.0) - 2026-10-17

### Added

- *(serial)* enumerate the ports without a system library

### Documentation

- *(serial)* state exactly what is compiled and what is tested

### Fixed

- *(serial)* the pty test is linux only; darwin rejects it with ENOTTY
- *(serial)* drop baud 0 on macos, which hangs up the line
- *(serial)* compile on windows, where there is no TIOCEXCL
