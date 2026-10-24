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

## Slice 2 — Wizard usability (items 11–20)
- [ ] 11. File picker opens in the folder of the path already typed
- [ ] 12. Drag-and-drop a setup file onto the pick step
- [ ] 13. Pick step shows the chosen file's size and a plain summary
- [ ] 14. Recent setup files (last 5) persisted and offered on the pick step
- [ ] 15. Wizard progress autosaved; reopening offers to resume the run
- [ ] 16. Enter = Continue, Escape = Back, on every step
- [ ] 17. Text-size option for the wizard (persisted), for readability
- [ ] 18. Name step pre-fills from capture metadata hints (item 4)
- [ ] 19. Name step shows inline validation (version shape, main-file name) as you type
- [ ] 20. Buttons to open the session and output folders on the Done step

## Slice 3 — Capture & agent depth (items 21–30)
- [ ] 21. Snapshot scan skips unreadable files and reports the skip count instead of failing the whole note
- [ ] 22. Services capture on Windows (`sc query`) fills `snapshot.services`, so service red flags fire from real captures
- [ ] 23. Agent heartbeat carries the current action ("looking through …") and scan progress %
- [ ] 24. Hash cache: unchanged files (same size + mtime) are not re-hashed between notes
- [ ] 25. Registry export failure names the hive that failed and retries once
- [ ] 26. `watch` agent command: snapshot when a folder has been quiet for N seconds (hands-free after-install note)
- [ ] 27. Host capture timeout: wizard/CLI warn when a requested note has not arrived in a set time
- [ ] 28. Start Menu shortcut targets harvested as main-executable candidates
- [ ] 29. Agent state writes are atomic (write-then-rename) so the host never reads a half-written state
- [ ] 30. Mode B packet: CLI generates a folder (agent + plain instructions) to run by hand in your own VM

## Slice 4 — Classification & review depth (items 31–40)
- [ ] 31. Findings de-duplicated across install + runtime passes for every finding type
- [ ] 32. Review shows red flags first, then fixes by confidence
- [ ] 33. Single files at the profile root classify as FilesMove candidates
- [ ] 34. Installed fonts classify as a distinct red flag
- [ ] 35. COM/ActiveX registrations (HKCR\CLSID) classify distinctly from file associations
- [ ] 36. Scheduled-task and firewall-rule style service names classify as red flags
- [ ] 37. Review decisions export/import as a file from the wizard
- [ ] 38. `review` CLI command: session folder → review.json, printable plain cards
- [ ] 39. Noise list grows: Delivery Optimization cache, Windows Search index stores (flagged, never dropped)
- [ ] 40. Every classification's `why` carries its evidence path, audited by test

## Slice 5 — Build & package depth (items 41–50)
- [ ] 41. PE architecture detection (x86/x64/ARM64) for the main executable, shown in the build report
- [ ] 42. Build warns when the main file is not a real Windows program file (no MZ/PE header)
- [ ] 43. Harvested `.ico` becomes `App\AppInfo\appicon.ico` automatically
- [ ] 44. Build refuses a non-empty output folder unless explicitly overridden, with a plain reason
- [ ] 45. Package size and file count in the build summary
- [ ] 46. Optional portable `.zip` of the finished tree alongside the package folder
- [ ] 47. Generator runner: verified PAL/Installer generator programs run when their installed paths are supplied
- [ ] 48. `[Launch]` gains the 64-bit program path line when the main file is x64-only
- [ ] 49. Specimen auto-seeding: build from a session maps harvested specimen files into DefaultData by the specimen plan
- [ ] 50. Deterministic manifest: SHA256SUMS sorted, stable across rebuilds of the same inputs

## Slice 6 — CLI & scripting (items 51–60)
- [ ] 51. `--json` for preflight, post-capture, and leaks outputs
- [ ] 52. `sessions` command lists session folders with phase, age, and next action
- [ ] 53. `doctor` command: preflight + builder tools + helper presence in one report
- [ ] 54. Documented exit codes (0 ok, 1 error, 2 validation/leaks found) applied consistently
- [ ] 55. Per-command help (`portableforge build --help`)
- [ ] 56. Host `snapshot` accepts several `--root` folders via the multi-root scan
- [ ] 57. `leaks --out <file>` writes the leak report as JSON
- [ ] 58. `validate --tree` can run from a details file *or* a built package's own appinfo
- [ ] 59. Quiet flag for snapshot/build progress in scripts
- [ ] 60. `classify` prints the technical rows behind a `--details` flag

## Slice 7 — Leak test & sandbox modes (items 61–70)
- [ ] 61. `verify` command: build, then judge the leak notes when a leak session exists
- [ ] 62. Leak report groups findings by kind with counts and one-line fixes
- [ ] 63. Wizard Test step runs the leak judge on notes dropped in the session folder
- [ ] 64. Careful-mode rollback executor (dry-run by default; applies the plan only with an explicit flag)
- [ ] 65. Preflight checks for a usable restore point (Mode C honesty) on Windows
- [ ] 66. Wizard Sandbox memory choice (2 / 4 / 8 GB) written into the .wsb
- [ ] 67. Wizard per-session networking toggle surfaced (default off)
- [ ] 68. `.wsb` generator supports the clipboard toggle in details view (default off)
- [ ] 69. Session cleanup command lists and removes old session folders on request
- [ ] 70. Leak-test box packet: `capture` notes named for the leak flow in one command

## Slice 8 — Platform & preflight depth (items 71–80)
- [ ] 71. Real free disk + memory figures on Windows (no new dependencies; OS queries from Rust)
- [ ] 72. Virtualisation-enabled check in preflight facts
- [ ] 73. Sandbox feature state read directly (DISM feature info) instead of "could not be checked"
- [ ] 74. Windows edition read covers IoT Enterprise naming variants
- [ ] 75. Preflight fix buttons copy the exact Settings path/command as text
- [ ] 76. `portableforge-agent` version handshake in state.json; host warns on mismatch
- [ ] 77. Session IDs are collision-proof (timestamp + app stem)
- [ ] 78. Long-path (>260) harvest entries reported plainly instead of failing copies
- [ ] 79. Unicode/space paths round-trip through .wsb, sessions, and the build (fixture tests)
- [ ] 80. Timezone-safe timestamps in session logs and build reports (UTC ISO-8601)

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
