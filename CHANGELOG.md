# Changelog

## [Unreleased]

## [0.2.1] — 2026-10-25

### Added
- **The quick test box's installer now travels with
  PortableForge.** Both downloads carry a `bundled` folder with
  the official Sandboxie-Plus installer — the maker's own file,
  untouched (its license allows passing the original along free
  of charge; the license texts and a plain note are in the folder
  too). On a computer with no test box yet, the safety check now
  offers "Install the quick test box now": PortableForge checks
  the installer's fingerprint against the value it was shipped
  with before opening it, and a copy that does not match is never
  opened. A "Check again" button re-reads the computer
  afterwards. The same action exists as a command, as always:
  `install-quick-box`. This makes the downloads bigger (about
  24 MB more) — that trade was chosen knowingly, for a setup with
  no hunting on the web.

## [0.2.0] — 2026-10-25

### Added
- **The quick test box (Sandboxie).** A whole new way to test,
  for computers where the safe test box cannot run — most
  importantly Windows Home, which does not include it at all.
  If Sandboxie-Plus is installed (a free program you install
  yourself; PortableForge only detects it and drives it), the
  safety check picks the quick test box on its own. The program
  runs on your computer, but everything it writes is caught in a
  sealed folder that is thrown away afterwards; it starts in
  seconds, and the notes, the review, and the build work exactly
  as before. It is not a separate computer — programs that
  install deep system parts may not install properly in it, and
  the review flags those, as it always has. New commands match
  the window, as always: `sbie-launch` and `sbie-capture`.

## [0.1.4] — 2026-10-25

### Fixed
- **Finish finishes.** On the last step, Finish quietly tidied up
  behind the scenes and left the window sitting there, so it
  looked like the click did nothing (field report). It now
  closes the window — the run is complete and its unfinished-run
  note is cleared, so the next launch starts fresh.
- **Long steps scroll.** The safety step with several fixes ran
  past the bottom of the window, and its last lines — the chosen
  way and the "use it anyway" choice — could not be reached at
  all. Step content now scrolls inside the window; the heading
  and the bottom buttons stay put.
- **Ticks and crosses are drawn, not typed.** The safety check's
  marks were characters the included font does not have, so every
  line showed the same hollow square and a pass looked like a
  fail. They are now drawn shapes — a tick you can tell from a
  cross at a glance.

## [0.1.3] — 2026-10-25

### Changed
- **The window looks finished now.** Same dark family colours and
  the same words everywhere, presented properly: the safety
  checks sit on a card instead of floating on black, the progress
  line at the top is slim and glides to each step instead of
  jumping, buttons and boxes have soft corners and room to
  breathe, the headings carry a small red rule, and Back /
  Continue are pinned at the bottom where your eye expects them —
  Continue is now the one filled red button on every step.

## [0.1.2] — 2026-10-25

### Fixed
- **The safety check misjudged a computer that can run the safe
  test box** (field report from the first installed run). Windows
  names your edition two different ways — the short name and the
  product name spell "Pro" differently — and the check only read
  one of them, so a capable computer was sent to Careful mode.
  It now reads both, and when the check still says no, its fix
  words name the edition your computer actually reported, so a
  wrong reading is visible instead of silent.
- **New choice on the safety step: "Use the safe test box
  anyway."** If you know your computer can run the test box, your
  word wins: PortableForge uses the box, and the report keeps an
  honest line saying the choice was yours. (A test box Windows
  says is truly switched off still says so — no choice can
  override the computer itself.)

### Changed
- Releases now carry exactly two downloads: the installer on its
  own, and one zip with the portable copy — no more bundle zip.

## [0.1.1] — 2026-10-25

### Fixed
- **The wizard no longer opens a console window beside it.** The
  v0.1.0 release binary was built as a console program, so starting
  the window also opened a black terminal window behind it (caught
  by checking the shipped file itself, before any install). The
  program is now built as a windowed program, and commands run
  from a terminal re-attach to that terminal first, so the
  command line keeps its output. The helper keeps its console —
  inside the test box that is where its work shows.

## [0.1.0] — 2026-10-25

### Added — first release packaging (2026-10-25)
- A proper installer joins the portable pair in the release zip:
  a per-user setup program (no admin needed, Start menu shortcut,
  clean uninstall) that installs both programs together — the
  helper always lands beside the main program, where it belongs.
  The version stays 0.1.0; 1.0.0 still waits for the on-machine
  runbook pass, as planned.

### The 100-enhancement programme — complete (2026-10-24)

Ninety-eight of the hundred planned improvements shipped in ten
slices, each one checked by the full test suite, published file by
file with fingerprint verification, and confirmed on a real Windows
build runner. The test suite grew from 47 checks to 92 along the
way. Two items wait, on purpose: the version stays at 0.1.0 until
the on-machine runbook has passed on a real Windows computer
(that pass is also where the README's screenshots come from) —
everything they need is built and waiting, including the release
machinery that turns a version tag into a downloadable zip with
fingerprints. A final audit traced every claim in the README to a
test, a build-runner result, or an on-machine result, and rewrote
the two claims that said more than the evidence allowed.

### Added — Slice 10: release & finish (2026-10-24)
- Tagging a version now builds the release by itself: a checked,
  fingerprinted zip (both programs, README, license, runbook) on a
  GitHub Release page, with the unsigned warning written into the
  notes. The README explains, in plain words, how to download a
  release and check its fingerprint before trusting it.
- First-run welcome: five plain lines on the first step explain the
  whole journey; they disappear once read and are held to the same
  no-jargon rule as every other line in the window.
- `portableforge --version` prints the version, the window's header
  shows it too, and the Done step can save the whole run — picks,
  decisions, build, leak verdict, and the log — as one plain text
  file.

### Added — 100-enhancement programme, Slice 9: proof, not promises (2026-10-24)
- The plain-language promise is now enforced on the wizard itself,
  not just the guide text: a test reads the wizard's own buttons and
  labels and fails if jargon creeps in. The registry reader is
  tested against a set of hostile, damaged files and must never
  crash on them; a fuzz-style sweep throws two hundred rounds of
  random path shapes at the comparing and sorting code; and a
  generated package tree is scanned back and diffed to prove the
  round trip loses nothing.
- A tiny made-up run — the fixture session in `examples/` — lets
  anyone rehearse the whole pipeline offline in five minutes
  (`docs/OFFLINE_REHEARSAL.md`), and a test keeps the fixture
  honest. New docs also record how the window treats eyes and
  keyboards (`docs/ACCESSIBILITY.md`, including the one place the
  palette falls short and why it is safe there) and what must be
  true before anything is called a release
  (`docs/RELEASE_READINESS.md`).

### Added — 100-enhancement programme, Slice 8: the safety check learns the computer (2026-10-24)
- The opening safety check now measures instead of guessing: real
  free space and memory, whether virtualisation is switched on,
  whether the safe test box feature is actually on (asked from
  Windows itself), and the IoT Enterprise editions are recognised by
  name. Anything that still cannot be checked says so, as before.
- Fixes you can copy: when a check fails and the fix is an exact
  place or command, the wizard offers a "Copy the fix" button and
  the command line prints it, so nobody has to retype
  `SystemPropertiesProtection` by hand.
- The helper program now introduces itself: if the helper in the
  test box is a different version than PortableForge, both the
  wizard and `agent-state` say so plainly. Run folders are named
  with the time they started, so starting the same conversion twice
  never collides. Files whose names are too long for Windows to copy
  (over the 260-letter path limit) are reported by name instead of
  failing the whole gathering. And every stamp PortableForge writes
  — run records, run logs, build reports — is now in UTC, so a log
  read later never needs timezone guesswork.

### Added — 100-enhancement programme, Slice 7: the leak test becomes a loop (2026-10-24)
- The leak test is now a proper round trip. `leak-packet` packs the
  finished copy, the helper, and plain instructions into one folder;
  the two notes it produces — leak-before and leak-after — are judged
  by the new `verify` command (build, then verdict) and by a new
  button on the wizard's test step. Leak reports group what was left
  behind — files, remembered settings, background helpers — each
  with a count and a one-line fix.
- Careful mode can now actually put things back: `rollback` rehearses
  the undo plan by default and only touches the computer with
  `--apply`, and even then it restores only what can be restored
  safely and names the rest as needing a person. The safety check
  also looks for a restore point and says so honestly when there is
  none or it cannot be checked.
- The test box is yours to size: memory (2, 4, or 8 GB), internet,
  and clipboard sharing are chosen on the wizard's clean-computer
  step and written into the box settings — internet and clipboard
  still default to off. Old run folders can be listed and cleared
  with `sessions-cleanup`.

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
