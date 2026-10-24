# Changelog

## [Unreleased]

### Added — the flow is now wired end to end (2026-10-24)
- **Build pipeline (new core `build` module):** one call turns the
  package details, the review's accepted fixes, the harvested program
  files, and the first-run seed files into a finished, validated
  package tree. Before this, generation was called with an empty fix
  list, so fixes accepted in review never reached the package.
- **The helper in the test box now takes the notes itself:** the host
  asks (a small file in the shared folder) and the helper answers with
  a full note — before, after installing, after trying out — scanning
  the places programs and settings actually land and, on Windows,
  capturing the remembered settings too. After the last note it copies
  the program's own folder out for the build step and signals that the
  capture is finished. New helper command `capture` takes one note on
  demand (finishing test, or a test computer you run yourself).
- **The guided window drives the whole flow:** "Start the test box"
  makes the work folder, copies in the setup file and the helper, asks
  for the clean note, and starts the box; "Check on the test box"
  follows the helper and fills in the "What it left behind" list by
  itself; the naming step now collects who makes the program, its main
  file name, and its version; "I am not sure" asks for the maker's
  download address and fingerprint; and the build step has a
  "Build my portable copy now" button that makes and checks the
  package in the session's output folder.
- **New commands for scripts:** `build` (session notes or a saved
  review + details → finished, checked package), `launch`,
  `request-snapshot`; `generate` now accepts `--review` so its
  starter rules carry the accepted fixes.

### Verified
- The full check suite (formatting, lint, all 45 tests, and a release
  build) passes on a real Windows runner for every push. This proves
  the code builds and its tests pass on Windows; the program's
  behaviour in a real test box is still verified only by the on-machine
  runbook in `docs/ON_MACHINE_RUNBOOK.md`.

### Added
- The host half of the live capture is wired: when the helper inside
  the test box finishes, PortableForge now reads its three notes back,
  works out both change lists, and has the review and the
  starting-settings seeds ready for the "What it left behind" step —
  including a plain error naming any note that is missing and which
  step makes it. (New `post-capture` command for testing.)
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
