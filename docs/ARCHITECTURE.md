# PortableForge — Architecture & UX Plan (v0.2, decisions locked 2026-10-23)

> **Name confirmed by SEAN 2026-10-23: PortableForge.** A Windows desktop wizard that
> automates the *entire* classic PortableApps.com packaging workflow — pre-scan,
> install, run, close, post-scan, classify, generate, build, and leak-test —
> inside a disposable sandbox, so the user's real machine is never touched by
> the app being packaged.
>
> Status labels used below: **Verified** = checked against a primary source in
> this document's Sources list. **Unverified** = design intent, not yet run on
> SEAN's Windows machine.

## TL;DR

You pick an installer. PortableForge launches it inside Windows Sandbox, snapshots
the sandbox before and after install and again after a first run/close,
works out from those diffs what the app installs and where it keeps its
settings, turns that into a PortableApps.com Format 3.9 package (appinfo.ini,
Launcher .ini, installer.ini, DefaultData), builds the launcher and the
`.paf.exe` installer with the official PortableApps.com generators, then
re-tests the finished portable app in a *fresh* sandbox and reports any
writes that leak outside the portable folder. Your host PC is untouched:
closing the sandbox discards everything in it (Verified — Microsoft Learn).

---

## 1. What exists today (why this is manual)

The community workflow, documented on the PortableApps.com forums (Verified),
is 10+ manual steps in a clean VM: RegShot → install → RegShot/compare →
hand-write a launcher → revert VM → install portable build → RegShot →
run/close → RegShot/compare → add registry/settings handling to the launcher
→ repeat until the scans are clean. PortableForge automates every one of those
steps and replaces RegShot with its own snapshot/diff engine.

Official pieces it orchestrates (it does **not** reimplement them):
- **PortableApps.com Format 3.9** (2026-03-02) — directory layout,
  `appinfo.ini`, `installer.ini`, icons, host-PC rules (Verified).
- **PortableApps.com Launcher (PAL) + Generator** — portablizes via
  `App\AppInfo\Launcher\<AppID>.ini`: `[Launch]`, `[DirectoriesMove]`,
  `[FilesMove]`, `[RegistryKeys]`, `[RegistryValueWrite]`, `[FileWriteN]`,
  `[Environment]`, etc.; NSIS core; no code needed (Verified).
- **PortableApps.com Installer + Generator** (3.9.x) — packages a finished
  app folder into a `.paf.exe`; `installer.ini` covers `[DownloadFiles]` /
  `AdvancedExtract` (online installers), `[DirectoriesToPreserve]`,
  optional components, languages (Verified).

## 2. Sandbox design — the core promise

### Mode A — Windows Sandbox (primary, true isolation)
- Disposable Hyper-V container built from the host Windows image; everything
  is discarded when it closes (Verified — Microsoft Learn).
- PortableForge generates a per-session `.wsb` (XML config, Verified format):
  `Networking=Disable` by default (opt-in per session), clipboard/printer/
  audio/video redirection disabled, `ProtectedClient=Enable`,
  `MemoryInMB` configurable, and three mapped folders only:
  - `input\` — host → sandbox, **read-only** (the original installer)
  - `tools\` — host → sandbox, **read-only** (`portableforge-agent.exe`)
  - `exchange\` — **read/write** (snapshots, harvested files, logs, heartbeat)
  `LogonCommand` boots the agent inside the sandbox.
- Requirements check (honest, shown in the wizard, not hidden):
  Windows 10 build 18342+ / Windows 11, **Pro / Enterprise / Education /
  IoT Enterprise only — Windows Sandbox is not available on Home**
  (Verified — Microsoft Learn policy/samples pages), virtualization on,
  ~1 GB disk / ≥4 GB RAM free. Preflight uses
  `Get-WindowsOptionalFeature -Online -FeatureName Containers-DisposableClientVM`
  and can offer the DISM enable command (admin, reboot) — it never enables
  silently.

### Mode B — Bring-your-own VM snapshot
User points PortableForge at a Hyper-V checkpoint / VirtualBox / VMware snapshot.
PortableForge generates the in-guest agent + its settings files (no scripts — §9.5); user reverts the snapshot
between the capture pass and the leak-test pass. Same diff/classify/generate
pipeline on the host.

### Mode C — Host Snapshot Sandbox (fallback, weakest guarantee)
For Home-edition machines with no VM: pre-scan on the host, create a System
Restore Point first (admin, verified created before continuing), install /
run / close / post-scan on the host, then a guided rollback driven by the
diff (uninstall + delete harvested leftovers + restore exported .reg keys).
**This mode is labelled in the UI, in plain language, as "monitored and
reverted — NOT isolated."** It never claims the host was untouched.
This reuses the snapshot/diff thinking from InstallScope/SetupScope.

Mode A is the default whenever it is available. The mode in use is stamped
into every session log and final report.

## 3. Components

```
portableforge (host, Rust, single portable exe)
├─ Wizard GUI (egui, black/red family look) — steps in §4
├─ CLI: portableforge new | validate | build | leak-test  (scriptable, same core)
├─ core/
│  ├─ preflight    edition/feature/disk/RAM/tool checks
│  ├─ sandbox      .wsb generation, launch, heartbeat watch, timeout/kill
│  ├─ snapshot     filesystem + registry + services/tasks inventory
│  ├─ diff         A→B and B→C diffs, noise filtering, hashing (SHA-256)
│  ├─ classify     diff → PAL/appinfo/installer decisions (§5)
│  ├─ generate     PAF 3.9 tree, appinfo.ini, Launcher .ini, installer.ini,
│  │               help.html, icon extraction (ICO 16–256 + PNGs)
│  ├─ build        pinned, SHA-256-verified PAL + Installer generators
│  └─ leaktest     fresh-sandbox run of the built portable app + report
└─ portableforge-agent (in-sandbox/in-guest, Rust, zero-dep single exe)
   ├─ state machine over exchange\state.json  (host writes commands,
   │  agent writes heartbeat + progress % / ETA)
   ├─ snapshot / watch-process-tree / quiescence detection
   └─ harvest (copy install tree + runtime specimens to exchange\)
```

Safety rules for the agent: it only ever writes to `exchange\` and the
sandbox's own disposable filesystem; the host `input\`/`tools\` mounts are
read-only at the .wsb level, so even a hostile installer cannot write back
through them (Verified mechanism — MappedFolder ReadOnly).

## 4. Wizard workflow (what the user sees)

1. **New Project** — pick installer (`.exe` / `.msi` / `.zip`), or a download
   URL (→ online-installer `[DownloadFiles]` package instead of bundling
   files the license may not allow redistributing), or an already-installed
   folder. Name/AppID/publisher/category/description/license questions are
   asked *later*, pre-filled from what the scans find.
2. **Preflight** — sandbox mode available? Tools present? Disk/RAM? Every
   failure gives the fix, not just an error. Progress % shown.
3. **Pre-scan** — sandbox boots, agent snapshots the clean system.
   User action: none. (Host: "Sandbox is ready" + live log.)
4. **Install** — user clicks through the installer **inside the Sandbox
   window** (silent switches `/S`, `/VERYSILENT`, `msiexec /quiet` are
   offered when the installer type is detected, never forced). Agent
   detects installer exit + 10 s write-quiescence.
5. **Run** — wizard lists candidate main EXEs found in the install diff
   (+ Start Menu shortcut targets), user picks one, launches it **in the
   sandbox**, uses the app, changes a setting or two when prompted
   (that is what makes the runtime diff reveal where settings live).
6. **Close** — agent watches the process tree exit; if a tray/background
   process lingers, it is named and the user can close it in the sandbox
   or pick "force close this process."
7. **Post-scan & Classify** — install diff + runtime diff, auto-classified
   (§5), shown as one review table: every file/key, proposed destination,
   confidence, Accept / Send to Data / Ignore. Known noise (Prefetch,
   MuiCache, recent-docs, thumbnail caches) is pre-ignored but visible.
8. **Metadata** — pre-filled form: versions pulled from the main EXE's
   version resource (PackageVersion must be `x.x.x.x` — Verified),
   category from the PAF 3.9 list, license booleans, icon extracted.
9. **Generate & Build** — PAF tree written, validator run, PAL Generator
   builds `<App>Portable.exe`, PA Installer Generator builds
   `<App>Portable_<ver>.paf.exe` + SHA-256. All output to a folder the
   user picks; nothing is uploaded anywhere.
10. **Leak Test** — one click (default on): fresh sandbox, install the
    `.paf.exe`, run the portable launcher, close, post-scan. Report:
    **Leaks: 0** or a list of writes outside the portable folder, each
    with a suggested PAL fix and a "fix & retest" loop.
11. **Done** — summary: package, installer, leak score, sandbox mode used,
    session log, and exactly what was *not* verified.

Every long step shows Progress % + ETA and can be cancelled; a cancelled
session can be resumed from its last completed snapshot (sessions are
folders on disk, not state in memory).

## 5. Classification rules (diff → portable package)

| Finding (runtime/install diff) | Generated handling |
|---|---|
| Files under the install dir | `App\<AppName>\` |
| `%APPDATA%\<Vendor>\<App>\**` created on run | `[DirectoriesMove]` to `%PAL:DataDir%` + specimen → `App\DefaultData\` |
| `%LOCALAPPDATA%`, `%USERPROFILE%\Documents\<App>` | Same pattern, per-path |
| `HKCU\Software\<Vendor>\<App>` written | `[RegistryKeys]` in Launcher .ini (+ `.reg` seed if defaults matter) |
| Config file containing `C:\Users\WDAGUtilityAccount`, drive letter, or install path | `[FileWriteN]` rewrite rules using PAL variables |
| `.lnk`, file associations, `HKCR` writes | `[Associations]` candidates — **off by default**, user opts in |
| `HKLM` writes, services, drivers, scheduled tasks | **Red flag panel:** not cleanly portable; options explained (PAL registry handling, `RequiresAdmin`, or "package as installer-only / abandon") — never silently dropped |
| Uninstall registry entry only | Noise — used for name/version metadata, then ignored |
| Single settings `.ini/.xml/.json` in install dir referencing `%APPDATA%` | `[FilesMove]` / env-var candidates, shown with evidence |

The sandbox username (`WDAGUtilityAccount`) and sandbox paths are treated
as canaries: any harvested config file mentioning them proves a hard-coded
absolute path that the launcher must rewrite — a check you simply cannot
get from a host install.

## 6. Output layout (PAF 3.9, Verified)

```
<AppName>Portable\
├─ <AppName>Portable.exe          (built by PAL Generator)
├─ help.html
├─ App\
│  ├─ AppInfo\  appinfo.ini · installer.ini · appicon.ico · appicon_*.png
│  │            Launcher\<AppName>Portable.ini
│  ├─ <AppName>\  (harvested program files)
│  └─ DefaultData\  (first-run settings specimens)
├─ Data\  (empty in the package)
└─ Other\ Source\ (launcher/installer source + readme) · Help\Images\
```

## 7. Standing defaults applied (SEAN)

Rust (host + agent), egui wizard in the black/red family look, MIT,
Windows 10+ supported where the sandbox mode allows it (Mode A needs
Pro/Ent Win10/11; Mode C keeps Home useful), offline-first, no telemetry,
no signing unless required, Progress % + ETA on long work, validator +
tests in CI, README with `## TL;DR` and real screenshots from SEAN's
machine only (capture list prepared, images land when supplied).
PAL / PA Installer are bundled *in release downloads only*, pinned +
SHA-256-verified, with their license attributions kept (GPL etc.) —
the InstallScope precedent.

## 8. Build slices (small, gated, in order)

- **S1 — Core + agent snapshot/diff (no GUI):** snapshot/diff engine,
  noise list, JSON session format; CLI `capture` in a manually-started
  sandbox. Gate: full test suite on synthetic fixture trees/hives.
- **S2 — Sandbox orchestration: CORE BUILT 2026-10-23 (preflight evaluation, session layout, launch plan, heartbeat read, harvest — tested off-Windows; live Sandbox run still Unverified).** Original scope: preflight, .wsb generation, agent state
  machine, heartbeat, harvest. Gate: on-machine run on SEAN's PC
  (Unverified until then).
- **S3 — Classify + Generate: BUILT 2026-10-23 (review model with severity defaults, specimen plan, template-verified Launcher generation).** Original scope: rules in §5, PAF 3.9 validator, tree
  generation from a captured session (works from S1 fixtures, so it is
  testable before S2 is machine-verified).
- **S4 — Wizard GUI: WIRED 2026-10-23 (safety step shows the real preflight report, changes step is a working review with acknowledgement gating, pick/sharing steps validate before continuing; live sandbox actions land with the S2 on-machine run).** Original scope: the 11 steps in §4 over S1–S3.
- **S5 — Build integration: PINS + VERIFICATION BUILT 2026-10-23 (real measured pins for PAL 2.2.2.1 + Installer 3.9.22 + Template 3.5.2; generator invocation plan; execution on Windows remains).** Original scope: pinned PAL + Installer generators, .paf.exe
  output + hashes.
- **S6 — Leak Test loop + Mode B/C fallbacks: ANALYSIS BUILT 2026-10-23 (leak judge + Mode C rollback planner, both tested; live loop + Mode B remain).** Original scope:

Estimated progress reporting follows the master prompt: % + ETA per slice.

## 9. Decisions (locked 2026-10-23)

1. **Name — PortableForge** (SEAN).
2. **First target — Muse's decision, delegated by SEAN: Windows Sandbox
   first.** Rationale: the brief's core promise is "host untouched," and
   only Mode A delivers it; Mode C first would lead with the weakest
   guarantee. Slice 1's snapshot/diff/classify core is mode-agnostic, so
   Home-edition Mode C (S6) reuses the same engine — nothing is thrown
   away, and no isolation claim is made that the tech cannot keep.
3. **Redistribution guardrail — YES, with the choice explicit** (SEAN).
   In the wizard this is a required, plain-language step, never a hidden
   default: *Bundled files* (only when the user asserts redistribution
   rights for the base app) vs *Online installer* (`[DownloadFiles]` with
   URL + SHA-256, files fetched at install time). The choice and the
   assertion are stamped into the session log and `Other\Source\`.
4. **GUI — egui** (SEAN), eframe + glow renderer (RoomForge lesson:
   wgpu with no backend feature panicked on a real machine), black/red
   family look.
5. **Dark UI only** (SEAN, 2026-10-23): one theme, the family tokens;
   no light theme. Status never carried by colour alone.
6. **100% Rust for authored code** (SEAN, 2026-10-23): host, core, CLI
   and the in-sandbox agent are all Rust; no PowerShell/batch/Python
   anywhere — the `.wsb` LogonCommand launches the agent binary
   directly. Named exceptions (orchestrated, never reimplemented):
   Windows Sandbox itself, `reg.exe`, and the official PortableApps.com
   Launcher/Installer generators — reimplementing those in Rust would
   break the Format compatibility the tool exists to produce.
7. **Fully guided, no-code experience** (SEAN, 2026-10-23): the wizard
   is a linear 12-step plain-language flow (core `guide.rs`); primary
   wording is mechanically jargon-checked by test; defaults pre-chosen;
   technical detail lives behind "Show details"; the classification
   review shows plain cards ("It keeps its settings in a folder…"),
   with the technical rows one toggle away; errors say what happened,
   why, and what to click.

## Sources

- PortableApps.com Format 3.9 — portableapps.com/development/portableapps.com_format
- PortableApps.com Launcher — portableapps.com/apps/development/portableapps.com_launcher
  and its manual — portableapps.com/manuals/PortableApps.comLauncher/
- PortableApps.com Installer 3.9.x release notes — portableapps.com
- Microsoft Learn — Windows Sandbox configuration (.wsb), sample configs,
  and Sandbox policy (editions) — learn.microsoft.com/windows/security/
  application-security/application-isolation/windows-sandbox/
- PortableApps.com forums — classic RegShot/VM packaging workflow threads
