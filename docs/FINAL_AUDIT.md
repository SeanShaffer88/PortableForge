# Final audit — every README claim, traced

Program-100 item 100, written at the programme's close (2026-10-24).
Each claim in `README.md` is traced to the evidence behind it: a
test in the gate, a CI run, or an on-machine result. Claims with no
evidence were rewritten until they matched reality (the fixes are
listed at the end).

| README claim | Evidence |
|---|---|
| Twelve plain steps | `guide::STEPS` has 12 entries; a test asserts the count and the jargon guard keeps the wording plain. |
| "never write code, never edit settings files" | The wizard drives every step; the only files a person ever picks are the setup file, folders, and save locations (wizard source). |
| The conversion install happens only inside the test box | The `.wsb` maps only the three folders; the agent runs only in the box; the host's own install path does not exist in the code (the careful mode is a separate, labelled path). Live behaviour: **Unverified** until the runbook passes. |
| About 2 GB free disk and memory needed | Preflight checks use exactly those thresholds (2,048 MB) in `evaluate_preflight`, unit-tested. |
| Builder programs' fingerprints checked before trusting them | `tools.rs` pins (version, size, SHA-256 measured from the real downloads); `find_verified_tool` refuses mismatches; the run commands verify first. Unit-tested. The makers' actual execution: **Unverified**. |
| Every finding shown as a plain card you can accept or question | Review model + wizard changes step; decisions gate the build (accepted fixes feed `build_package`). Unit-tested. |
| Package built to PortableApps.com Format 3.9 | Generation checked against the official Template 3.5.2 (read in full during the original build); INI conformance is validated by `check_launcher_ini` and the tree validator in the gate. |
| Finished copy tested in a fresh box; anything left behind reported | Leak judge (`leaktest`) is unit-tested and exercised end-to-end in the offline rehearsal; the in-box run itself: **Unverified**. |
| Only three folders shared, two read-only; no internet unless turned on | `.wsb` generator test counts the read-only mappings (2 of 3) and asserts networking/clipboard default to off. |
| "PortableForge itself is offline, collects nothing" | No telemetry or network code exists outside the explicit, fingerprint-checked builder-program download. Source-level claim. |
| Careful mode on Home, labelled not sealed off | `SandboxMode::describe` wording is asserted by test; the honesty line appears in the wizard, CLI, and rollback output. |
| Sharing answers change how the copy is built | The distribution guardrail is code (`Distribution::check_guardrail`); "not sure" maps to the online-fetch design with URL + fingerprint required. Unit-tested. |
| The two official makers named by exact version | Pins measured from the real downloads: starter maker 2.2.2.1, setup maker 3.9.22 (`tools-pins/PINS.md`). |
| Full flow wired end to end (Status section) | The offline rehearsal (`docs/OFFLINE_REHEARSAL.md`) runs capture → review → build → leak judgment from the checked-in fixture on any machine; CI runs the full suite on windows-latest for every push (latest programme run: Slice 9, CI #96). |
| Windows runtime status | The README says plainly it is **not yet verified on a real Windows machine** — true as of this audit; the runbook is the only path to changing that sentence. |

## Claims fixed by this audit

- "you get one finished file" (TL;DR) and "one finished file" in
  step 10 — the build produces the finished package *folder*; the
  single file comes from the official setup maker afterwards.
  Reworded to "the finished portable copy" / "the finished package".
- "with the official builder programs doing the final assembly" —
  they run only when asked (`tools run-installer` /
  `tools run-launcher`), not automatically inside the build step.
  Reworded to say exactly that.
