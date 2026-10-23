# Changelog

## [Unreleased]

### Added
- Safety-check step now judges the real computer (Windows edition, build,
  test-box presence) and shows each check in plain words with its fix, and
  honestly offers Careful mode when the test box is unavailable.
- Session folders, test-box launch plan, and result gathering are built:
  everything a run makes lives in one folder that can be picked up again.
- The "What it left behind" step is wired: fixes start on, file-type
  claims start off, and big warnings need a tick before you can go on.
- The finished-copy test has a judge: anything a portable copy changes
  outside its own folder is reported in plain words with a suggested fix.
- Careful mode now has a written put-back plan: what it added gets
  deleted, what it changed gets restored from the before-note, and
  anything unusual is listed for a person to check — never guessed at.
- The two official PortableApps.com builder programs are recorded by
  exact version, size, and fingerprint, measured from the real downloads
  (starter maker 2.2.2.1, setup maker 3.9.22); a file that does not match
  its record is refused.

### Fixed
- The starter rules we generate now match the official example package
  exactly (checked against PortableApps.com's own template, downloaded
  and read in full): settings folders and remembered settings use the
  template's key-and-value order, and the path-updating rules use the
  template's three-step form, most specific first.

## [0.1.0] — 2026-10-23

First slice — the thinking parts, built and tested off-Windows.
**Windows runtime: Unverified** (not yet run on a real Windows machine).

### Added
- Twelve-step guided flow defined in plain language in the core library,
  with a test that keeps jargon out of the wording people actually read.
- Change-spotting engine: notes every file (with a fingerprint check)
  and every remembered setting before and after, then lists exactly
  what changed. Background Windows housekeeping is marked as such,
  never quietly removed from the record.
- Understanding step: turns those changes into the fixes a portable
  copy needs — settings folders and files to carry, remembered settings
  to carry, files that wrote down one computer's details, and clear
  warnings for the few changes a portable copy cannot honestly make.
- Package making: builds and checks a PortableApps.com Format 3.9
  package (the app's details page, its starter rules, and the setup
  maker's settings), including the sharing question — copies are either
  whole programs you have said may be shared, or they fetch the
  original from its maker with a fingerprint check. Anything else
  cannot be built.
- Test-box settings file maker (Windows Sandbox), sealed off by
  default: no internet, no clipboard or printer sharing, only three
  folders shared, two of them read-only.
- A helper program that works inside the test box, written in Rust
  like everything else, with a heartbeat so the main window always
  knows it is alive.
- The main window: a dark, black-and-red guided wizard you can walk
  from start to finish, plus the same steps as commands for scripts
  and testing.
