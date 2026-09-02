# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Project scaffolding: filled the architecture overview, milestone-based
  development plan, and future-extensions sections in `Urbix_Project.md`.
- Crate skeleton (`Cargo.toml`) and module map under `src/`, each file
  carrying a doc-comment header describing its role in the architecture:
  `api`, `building`, `cache`, `chunk`, `config`, `data`, `engine`, `ffi`,
  `hash`, `interior`, `region`, `street`, `zones`, plus `lib.rs` and `main.rs`.
- Test, benchmark, example, and documentation scaffolding directories.
- C header placeholder at `include/urbix.h` (generated from FFI in Milestone 7).
- `.gitignore` for Rust build artifacts.

## [0.1.0] — 2026-09-02

### Added

- Initial project document `Urbix_Project.md` describing the objectives,
  architecture, world-generation design, versioning conventions, and
  documentation standards.
- Initial `README.md` with a one-line project description.
- Local git identity configured (`user.name` / `user.email`) for this
  repository.
