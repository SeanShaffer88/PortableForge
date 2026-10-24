# PortableForge — 100-enhancement program

Approved by SEAN 2026-10-24 ("add 100 enhancements"). Work proceeds in
10 slices of 10, in the order below (recommended by impact × effort),
each slice gated on the full suite (fmt + clippy host and windows-msvc
+ all tests + Windows CI) and published with blob verification before
the next begins. Windows runtime for new work stays **Unverified**
until SEAN's on-machine pass says otherwise.

Status key: `[ ]` pending · `[~]` in progress · `[x]` done (gated + published)

## Slice 1 — Correctness, capture intelligence, build provenance (items 1–10) — DONE 2026-10-24
- [x] 1. Escape HTML in generated `help.html` (names/descriptions are user text)
- [x] 2. Sanitise INI values in generated files (strip CR/LF so a crafted name can't inject INI lines)
- [x] 3. Canary content scan in post-capture: harvested text files are read and absolute-path findings join the review
- [x] 4. Uninstall-metadata extraction: DisplayName / Publisher / DisplayVersion from the install note pre-fill the naming step
- [x] 5. Build report: `Other\Source\build-report.json` in every built package (files, counts, validation result)
- [x] 6. `Other\Source\SHA256SUMS.txt` manifest of the finished package's authored + program files
- [x] 7. `agent-state` CLI command: agent phase, heartbeat age, next host action, stale-heartbeat warning
- [x] 8. `build --json` machine-readable build outcome for scripts
- [x] 9. Theme contrast test: WCAG ratios for the dark palette asserted in the gate
- [x] 10. Post-capture saves its artefacts (`review.json`, both diffs) into the session folder automatically

Slice 1 gate: fmt clean; clippy clean on host and `x86_64-pc-windows-msvc`; **53/53 tests**; off-Windows smoke re-run green (post-capture artefacts, `build --json`, provenance files, `agent-state`).

## Slice 2 — Wizard usability (items 11–20) — DONE 2026-10-24
- [x] 11. File picker opens in the folder of the path already typed
- [x] 12. Drag-and-drop a setup file onto the pick step
- [x] 13. Pick step shows the chosen file's size and a plain summary
- [x] 14. Recent setup files (last 5) persisted and offered on the pick step
- [x] 15. Wizard progress autosaved; reopening offers to resume the run
- [x] 16. Enter = Continue, Escape = Back, on every step
- [x] 17. Text-size option for the wizard (persisted), for readability
- [x] 18. Name step pre-fills from capture metadata hints (item 4)
- [x] 19. Name step shows inline validation (version shape, main-file name) as you type
- [x] 20. Buttons to open the session and output folders on the Done step

Slice 2 notes: the wizard's memory lives in one settings file
(`<home>\PortableForge\wizard-settings.json`, new core `settings`
module — recents, text size, run draft; written atomically). Keyboard
shortcuts never fire while typing in a box. Name-step pre-fill happens
once per capture and only into fields the person left empty. Gate:
fmt clean; clippy clean host + windows-msvc; **60/60 tests** (58 core
+ 2 wizard). Windows GUI runtime stays Unverified until SEAN's pass.

## Slice 3 — Capture & agent depth (items 21–30) — DONE 2026-10-24
- [x] 21. Snapshot scan skips unreadable files and reports the skip count instead of failing the whole note
- [x] 22. Services capture on Windows (`sc query`) fills `snapshot.services`, so service red flags fire from real captures
- [x] 23. Agent heartbeat carries the current action ("looking through …", files checked so far — a total can't be known mid-scan, so no invented %)
- [x] 24. Hash cache: unchanged files (same size + mtime) are not re-hashed between notes
- [x] 25. Registry export failure names the hive that failed and retries once
- [x] 26. `watch` agent command: snapshot when a folder has been quiet for N seconds (hands-free after-install note)
- [x] 27. Host capture timeout: wizard/CLI warn when a requested note has not arrived in a set time
- [x] 28. Start Menu shortcut targets harvested as main-executable candidates
- [x] 29. Agent state writes are atomic (write-then-rename) so the host never reads a half-written state
- [x] 30. Mode B packet: CLI generates a folder (agent + plain instructions) to run by hand in your own VM

Slice 3 notes: snapshots gained `skipped_files` and `capture_warnings`
(both defaulted, so older notes still read). The hash cache lives in
the exchange folder (`hash-cache.json`) and is the standard
size+mtime trade-off — documented in the module. The wizard pre-fills
the main file from the first shortcut candidate when it's empty, and
both the wizard and `agent-state` warn after 120 s of waiting for a
note. Item 28 landed as shortcut *names* (the honest signal available
without parsing .lnk targets). Gate: fmt clean; clippy clean host +
windows-msvc; **65/65 tests**; off-Windows smoke green (note sequence
with cache, pending-request warning, Mode B packet, `watch` settling
at change+quiet with both files in the note).

## Slice 4 — Classification & review depth (items 31–40) — DONE 2026-10-24
- [x] 31. Findings de-duplicated across install + runtime passes for every finding type
- [x] 32. Review shows red flags first, then fixes by confidence
- [x] 33. Single files at the profile root classify as FilesMove candidates
- [x] 34. Installed fonts classify as a distinct red flag
- [x] 35. COM/ActiveX registrations (HKCR\CLSID) classify distinctly from file associations
- [x] 36. Scheduled-task and firewall-rule registrations classify as distinct red flags
- [x] 37. Review decisions export/import as a file from the wizard
- [x] 38. `review` CLI command: session folder → review.json, printable plain cards
- [x] 39. Noise list grows: Delivery Optimization cache, Windows Search index stores (flagged, never dropped)
- [x] 40. Every classification's `why` carries its evidence path, audited by test

Slice 4 notes: the de-duplication is one final pass over the
classifier output (plus an up-front service merge), so a finding seen
by both passes — or a service listed twice — reaches the review
exactly once. Ordering lives in `Review::from_classifications`, so
the wizard and the new `review` CLI agree. Item 36 landed as
registry-registration detection (the Task Scheduler store and the
firewall policy store), which is where those commitments actually
appear in a capture. COM registrations under HKLM\SOFTWARE\Classes
are checked before the generic HKLM branch so they keep their own
kind. The evidence audit (item 40) builds one finding of every kind
and asserts each `why` names the path, key, or service it was decided
from. Gate: fmt clean; clippy clean host + windows-msvc; **70/70
tests**; `review` CLI smoke green on a real capture session.

## Slice 5 — Build & package depth (items 41–50) — DONE 2026-10-24
- [x] 41. PE architecture detection (x86/x64/ARM64) for the main executable, shown in the build report
- [x] 42. Build warns when the main file is not a real Windows program file (no MZ/PE header)
- [x] 43. Harvested `.ico` becomes `App\AppInfo\appicon.ico` automatically
- [x] 44. Build refuses a non-empty output folder unless explicitly overridden, with a plain reason
- [x] 45. Package size and file count in the build summary
- [x] 46. Optional portable `.zip` of the finished tree alongside the package folder
- [x] 47. Generator runner: verified PAL/Installer generator programs run when their installed paths are supplied
- [x] 48. `[Launch]` gains the 64-bit program path line when the main file is x64-only
- [x] 49. Specimen auto-seeding: build from a session maps harvested specimen files into DefaultData by the specimen plan
- [x] 50. Deterministic manifest: SHA256SUMS sorted, stable across rebuilds of the same inputs

Slice 5 notes: the PE reader lives in `snapshot.rs` (`pe_arch`,
header-only, unit-tested on crafted headers for all three families).
The build reads the main file's header before generating, so an x64
program gets `ProgramExecutable64=` in its Launcher settings from the
same build. The zip writer is a small stored-entries writer in
`build.rs` (fixed 1980 date + sorted entries, so the same tree zips
to the same bytes; validated against Python's zipfile incl. CRC).
Item 47 landed as `tools run-installer` / `tools run-launcher`: the
pinned download is fingerprint-verified first, the maker program is
taken from `--generator` when supplied (else found in the tools
folder), and it runs only on Windows — elsewhere the exact command
is printed. Item 49 seeds from `exchange\specimens\` first, then the
harvest by file name; specimens whose contents were never captured
are reported as noted-but-unseeded, never silently dropped. Also
landed in this slice, beyond the ten: pre-flight metadata check
listing *every* problem before anything is written (with new
empty-publisher/homepage warnings and a near-limit description
warning at 480 of 512), license-file detection into `App\AppInfo\`,
a supplied `--icon`, per-folder size breakdown in the build report,
`Data\` preservation counting on rebuilds, richer `help.html`
(version, publisher, "Made with PortableForge"), and a Launcher INI
spelling check against the Template 3.5.2 sections/closed key sets.
Gate: fmt clean; clippy clean host + windows-msvc; **79/79 tests**;
CLI smokes green (x64 build + zip + guard + seeding).

## Slice 6 — CLI & scripting (items 51–60) — DONE 2026-10-24
- [x] 51. `--json` for preflight, post-capture, and leaks outputs
- [x] 52. `sessions` command lists session folders with phase, age, and next action
- [x] 53. `doctor` command: preflight + builder tools + helper presence in one report
- [x] 54. Documented exit codes (0 ok, 1 error, 2 validation/leaks found) applied consistently
- [x] 55. Per-command help (`portableforge build --help`)
- [x] 56. Host `snapshot` accepts several `--root` folders via the multi-root scan
- [x] 57. `leaks --out <file>` writes the leak report as JSON
- [x] 58. `validate --tree` can run from a details file *or* a built package's own appinfo
- [x] 59. Quiet flag for snapshot/build progress in scripts
- [x] 60. `classify` prints the technical rows behind a `--details` flag

Slice 6 notes: exit code 2 is carried by a small process-wide slot
the judging commands (build, validate, leaks, doctor) set instead of
failing — scripts can now tell "the tool broke" (1) from "the answer
is no" (2), and the codes are documented in the help text and every
per-command help that can produce them. `validate --tree` without a
details file rebuilds the few fields the tree check needs from the
package's own appinfo.ini (the main program's name comes from the
ExtractIcon path). The host `snapshot` multi-root form reuses the
agent's own multi-root scan, cache and all. Gate: fmt clean; clippy
clean host + windows-msvc; **79/79 tests**; CLI smokes green for
every item (help, JSON shapes parse, exit codes 0/1/2 observed,
multi-root note contents, sessions listing, doctor report).

## Slice 7 — Leak test & sandbox modes (items 61–70) — DONE 2026-10-24
- [x] 61. `verify` command: build, then judge the leak notes when a leak session exists
- [x] 62. Leak report groups findings by kind with counts and one-line fixes
- [x] 63. Wizard Test step runs the leak judge on notes dropped in the session folder
- [x] 64. Careful-mode rollback executor (dry-run by default; applies the plan only with an explicit flag)
- [x] 65. Preflight checks for a usable restore point (Mode C honesty) on Windows
- [x] 66. Wizard Sandbox memory choice (2 / 4 / 8 GB) written into the .wsb
- [x] 67. Wizard per-session networking toggle surfaced (default off)
- [x] 68. `.wsb` generator supports the clipboard toggle in details view (default off)
- [x] 69. Session cleanup command lists and removes old session folders on request
- [x] 70. Leak-test box packet: `capture` notes named for the leak flow in one command

Slice 7 notes: the leak flow now has one set of note names everywhere
(`leak-before.json` / `leak-after.json` in the session's exchange
folder) — the packet makes them, `verify` judges them after a build,
and the wizard's test step judges the same notes in place. The
rollback executor deletes only files the run added (in the plan's
already-guarded places), restores settings whose old values are in
restorable forms (text, dword, binary — other forms are reported,
never guessed), and always reports changed files and helpers as
needing a person. The restore-point check never blocks the safe test
box; it exists so careful mode is never chosen blind, and on Windows
it reads `vssadmin list shadows` (no answer — e.g. no elevation —
stays "could not be checked"). The wizard's box choices (memory,
internet, clipboard) are per-run and start from the safe defaults
every launch. Gate: fmt clean; clippy clean host + windows-msvc;
**82/82 tests**; smokes green for every item (.wsb clipboard/memory
rendering, grouped leak output, verify end-to-end with exit 2,
packet contents, rollback rehearsal, cleanup list + remove,
restore-point line).

## Slice 8 — Platform & preflight depth (items 71–80) — DONE 2026-10-24
- [x] 71. Real free disk + memory figures on Windows (no new dependencies; OS queries from Rust)
- [x] 72. Virtualisation-enabled check in preflight facts
- [x] 73. Sandbox feature state read directly (DISM feature info) instead of "could not be checked"
- [x] 74. Windows edition read covers IoT Enterprise naming variants
- [x] 75. Preflight fix buttons copy the exact Settings path/command as text
- [x] 76. `portableforge-agent` version handshake in state.json; host warns on mismatch
- [x] 77. Session IDs are collision-proof (timestamp + app stem)
- [x] 78. Long-path (>260) harvest entries reported plainly instead of failing copies
- [x] 79. Unicode/space paths round-trip through .wsb, sessions, and the build (fixture tests)
- [x] 80. Timezone-safe timestamps in session logs and build reports (UTC ISO-8601)

Slice 8 notes: the Windows fact collection now asks the operating
system directly — free disk/memory and the virtualisation flag come
from kernel32 (`GetDiskFreeSpaceExW`, `GlobalMemoryStatusEx`,
`IsProcessorFeaturePresent(21)`), the Sandbox feature state from
DISM's own feature info, and restore points from vssadmin (slice 7).
Each fact still degrades to "could not be checked" rather than a
guess. The edition logic moved into the core as one tested function
covering the IoT Enterprise spellings (`IoTEnterprise`,
`IoTEnterpriseS`, `IoTEnterpriseK`), LTSC (`EnterpriseS`), and
Professional Education, with the registry ProductName as a fallback
witness. Every failed preflight check that has an exact fix now
carries it as copyable text — the wizard shows a "Copy the fix"
button, the CLI prints "Copy this: …". Session stamps: the record
carries `created_utc`, session log lines are prefixed with a UTC
stamp in `Session::say`, and the build report carries
`built_at_utc`, all from the new dependency-free `timeutil` module.
Runtime note: the Windows-only collectors compile under the
windows-msvc gate and CI, but their on-machine readings join the
runbook's verification list — they are Unverified until SEAN's pass.
Gate: fmt clean; clippy clean host + windows-msvc; **87/87 tests**.

## Slice 9 — Quality, tests, accessibility (items 81–90)
- [ ] 81. Malformed .reg inputs never panic the parser (hostile-input test set)
- [ ] 82. Generated-tree snapshot/diff round-trip property test on synthetic trees
- [ ] 83. Jargon guard extended to the wizard's own button and label strings
- [ ] 84. Accessibility notes doc: palette contrast table, focus order, text scaling behaviour
- [ ] 85. Full-workspace gate script equivalent documented in AGENTS.md (already policy; make it one copy-paste block)
- [ ] 86. Test count and coverage of core modules reported in the tracker per slice
- [ ] 87. Example fixture session (tiny, synthetic) checked in for the runbook's offline rehearsal
- [ ] 88. Offline rehearsal doc: run the whole pipeline from the fixture without Windows Sandbox
- [ ] 89. Fuzz-style loop in tests: random path shapes through normalise/diff/classify never panic
- [ ] 90. Release-readiness checklist doc (gates, runbook, screenshots, changelog)

## Slice 10 — Release & finish (items 91–100)
- [ ] 91. Version 1.0.0 once the on-machine runbook passes; crates + docs aligned
- [ ] 92. Release workflow: tag → Windows build → portable zip + SHA-256 → GitHub Release
- [ ] 93. Release zip contains both programs, README, LICENSE, and the runbook
- [ ] 94. Download-and-verify instructions in the README (hash check in plain words)
- [ ] 95. README screenshots wired from SEAN's real captures (list already prepared)
- [ ] 96. `portableforge --version` and an in-window version line
- [ ] 97. First-run welcome explains the whole journey in five plain lines (guide-checked)
- [ ] 98. Session log export from the Done step (one file, plain text)
- [ ] 99. Program close-out: tracker fully checked, CHANGELOG programme summary
- [ ] 100. Final audit: every README claim traced to a test, a CI run, or an on-machine result
