# Changelog

## [Unreleased]

### Added — 100-enhancement programme, Slice 6: the command line grows up (2026-10-24)
- For scripts and careful checking: `preflight`, `post-capture`, and
  `leaks` can all answer in JSON (`--json`), and `leaks --out` saves
  its report to a file. Commands now end with documented exit codes —
  0 fine, 1 something went wrong, 2 the run worked but a check found
  problems — so a script can tell "the tool broke" from "the answer
  is no". `--quiet` silences the progress chatter on `snapshot` and
  `build`, and every command has its own help (`portableforge build
  --help`).
- Two new commands: `sessions` lists your earlier runs with their
  step, how long since they were touched, and what should happen
  next; `doctor` gives one health report — this computer, the maker
  programs, and whether the helper program is in place. The host
  `snapshot` command can read several folders into one note, and
  `validate --tree` can check a built package from its own paperwork
  alone, with no details file needed. `classify --details` shows the
  technical row behind each plain card.

### Added — 100-enhancement programme, Slice 5: building gets deeper and safer (2026-10-24)
- Before a build writes anything, it now lists *every* problem with
  the details at once — and it refuses to build into a folder that
  already holds something else, with a plain reason (a folder from
  an earlier build is fine; that is a rebuild, and your saved-data
  folder is always kept exactly as it was, and the summary says so).
- PortableForge now reads the main program file's own header: the
  build report names its processor family (x86, x64, or ARM64), an
  x64 program gets the matching 64-bit line in its starter settings,
  and a "main file" that isn't really a program file earns a plain
  warning. A picture file found among the program's files becomes
  the package picture automatically, the program's license file is
  copied into the package, and the summary reports the total size
  and file count with a per-folder breakdown in the build report.
- `build --zip` also leaves a zip copy of the finished package;
  `build --session` now plants the captured first-run files into
  the package's starting settings by itself (and says plainly which
  ones weren't captured). Two new commands — `tools run-installer`
  and `tools run-launcher` — run the official PortableApps.com maker
  programs (fingerprint-checked first) on a finished package, only
  when you ask. The help page now shows the version and publisher
  and a "Made with PortableForge" line.

### Added — 100-enhancement programme, Slice 4: the review list gets sharper (2026-10-24)
- The same change can no longer appear twice in the "What it left
  behind" list when it was seen both while installing and while
  trying the program out. Warnings now come first in the list, then
  the fixes surest first — and every card's explanation names the
  exact file, settings area, or service it was decided from.
- Four things that used to blur into the crowd now get their own
  clear warnings: fonts installed into the Windows Fonts folder,
  helper components registered for other programs to use (COM/
  ActiveX), scheduled tasks, and firewall rules. A single file an
  app drops loose in your profile folder is now spotted and carried
  as a file, not a folder. Two more kinds of Windows housekeeping
  (update delivery cache, search index stores) are recognised as
  background noise — still listed, never hidden.
- The choices made in the review can be saved to a file and loaded
  back from the window, and a new `portableforge review
  <work folder>` command prints the whole list as plain cards and
  saves the choices file without opening the window.

### Added — 100-enhancement programme, Slice 3: capturing gets deeper and hardier (2026-10-24)
- One locked or unreadable file can no longer ruin a whole note:
  the scan now leaves it out, counts it, and the note says how many
  were left out. Notes after the first are much faster — files whose
  size and time are unchanged aren't read again (their fingerprint
  is remembered in the work folder).
- The helper now also records the machine's program services on
  Windows, so the "installs a service" warning can fire from real
  captures; a settings area that can't be read is retried once and
  then named in the note instead of vanishing quietly.
- While a note is being taken, the helper's heartbeat says what it
  is looking through and how many files it has checked, and its
  notes and heartbeat are written in one move so the wizard never
  reads a half-written file.
- New helper command `watch`: start it before running a setup
  program and it takes the after-installing note by itself once
  things have been quiet for a while. If a requested note hasn't
  arrived after two minutes, the wizard and the `agent-state`
  command say the test box may have been closed.
- Shortcuts the install put on the Start Menu are reported after a
  capture, and the wizard uses the first one to suggest the
  program's main file. New `mode-b` command makes a packet — the
  helper plus plain instructions — for testing inside your own
  virtual machine.

### Added — 100-enhancement programme, Slice 2: the window gets easier to use (2026-10-24)
- The file chooser now opens in the folder of the path you already
  typed, and you can drag the setup file straight onto the first
  step. The first step also says plainly what the path points at —
  the file's size and name, or that the path is a folder, or that no
  file is there yet — and offers the setup files you used lately
  (the last five, remembered between runs).
- Your progress is saved as you go. Close the window mid-run and
  reopening offers to pick the run up where you left it; finishing a
  run clears it.
- Pressing Enter continues and Escape goes back, everywhere except
  while you are typing in a box. A Text size choice (Standard /
  Large / Extra large) in the header is remembered between runs.
- After a capture, the naming step fills itself in from the
  program's own name, maker and version tags — only into boxes you
  left empty, and you can always change them. The naming step also
  points out problems as you type (a version that isn't four
  numbers, a main file that looks like a whole path), and the final
  step gained buttons that open the folder with your portable copy
  and the work folder.

### Added — 100-enhancement programme, Slice 1: correctness, capture intelligence, build provenance (2026-10-24)
The programme plan and tracker live in `docs/PROGRAM-100.md`.
- Generated `help.html` now escapes the program name, description and
  maker, and generated INI values can no longer smuggle in extra lines
  through line breaks in a name or description.
- When a capture is read back, any text files the helper harvested are
  checked for addresses baked in from the test box; anything found is
  added to the fix list. The program's own name, maker and version
  tags (from its uninstall registration) are read too, saved
  artefacts (`review.json`, both change lists, the specimen plan) now
  land in the session folder automatically, and the naming step will
  pre-fill from those tags in Slice 2.
- Every build now leaves a receipt in the package:
  `Other\Source\build-report.json` (what was written, copied, and
  whether the package passed its checks) and
  `Other\Source\SHA256SUMS.txt`, a fingerprint list of the finished
  package's files.
- New `agent-state` command shows what the helper in the test box is
  doing, how long since it last reported in, and what should happen
  next — with a warning if it has gone quiet. `build` learned `--json`
  for scripted use, and the dark palette's readability ratios are now
  asserted by a test in the gate.

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

### Fixed
- **The setup-file box now has a Browse… button** (field report from
  the first on-machine run: the window opens and runs, but the first
  step asked for a typed path with no way to pick the file). It opens
  the computer's own file chooser, filtered to setup files
  (.exe/.msi/.zip); typing or pasting the path still works too.

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
