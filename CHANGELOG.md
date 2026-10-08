# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0] - 2026-10-08

### Added
- `cacc remove <account>` (alias `rm`, `-y` to skip the prompt) deletes a saved account
- `cacc rename <old> <new>` renames an account and moves its stored credential
- `cacc doctor` checks tools, the live login and each saved account's credential

### Changed
- Account names can no longer contain whitespace or slashes, be a bare number, or match another account's name or email

## [0.1.0] - 2026-10-08

### Added
- `cacc list`, `cacc set <account>`, `cacc switch` (fzf picker), `cacc usage`
- `cacc login` and `cacc add` to save accounts
