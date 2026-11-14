# Decisions — PortableForge

## 2026-11-14 — Resume an autonomous app package

Save one paused app package independently of setup drafts and profile approvals.
Store the original source selection and editable decisions locally. Resume through
the same source inspection and build checks, using new temporary staging. Require
explicit redistribution permission on every resumed run.
Keep applied artwork in the same atomic save. Restore it when the saved start
file is still eligible. Saved artwork does not depend on imported image paths.

## 2026-11-12 — Package ZIPs and selected app files

Treat each drop as one source for one app. A ZIP or a same-folder selection of
loose files is prepared in private temporary work and passed to the existing
folder scanner. Preserve the included files and their layout. A setup file
continues through capture. Avoid reading sibling files the user did not select.

Guided and Autonomous use the same review, tool verification, sharing check,
and atomic build. Autonomous starts a build when its required decisions have
been made. Select a mode on the source screen; permission to include files
applies to the current source only.

## 2026-11-12 — Build from an unpacked app folder

Use the folder scan to propose an entry point and read app details. Require an
explicit decision before bundling files. Preserve the selected directory layout
and generate launcher settings for the selected executable. Findings about
likely fixed paths are advisory because each app interprets its settings
differently.

Run the pinned builders in private staging. Publish the verified installer and
its receipt together to a new output folder. The wizard reports static build
evidence separately from installation or behavior after moving the app.
[Folder workflow](docs/FOLDER_FIRST.md)

## 2026-11-12 — Bundle the portable test box files

This replaces the October 25 setup executable bundle. Release packaging
extracts the pinned official Sandboxie-Plus 1.18.8 x64 payload before it
stages PortableForge. The wizard can open the folder in Explorer.
Administrator prompts and the service lifecycle belong to the vendor
manager that the user opens from a protected local copy. Verified
capture and installer testing attach to a compatible running engine
after checking its active service and driver identities.

## 2026-11-12 — Seal generated package artwork with the profile

The artwork editor works on the current draft. Preview and packaging use
the same image transforms. Saving a profile generates the app icon files
and optional installer left pane before the package is fingerprinted. A
saved profile carries the generated assets, so a later build has no
dependency on the original image files. Applying artwork takes effect for
the current wizard session, and editing stops when the profile is saved.

## 2026-11-12 — TesselGUI revision 2 for the wizard

The wizard uses the GitHub palette and workflow patterns from TesselGUI
revision 2. This replaces the October 23 dark-only decision. The
generated Rust token file is copied verbatim from the pinned MIT code
export. Both projects ship the same copyright and MIT license text.
[Token source](https://github.com/SeanShaffer88/TesselGUI/blob/a689a5e3bfadf9e1e77a3acea826bea5cc717e94/dist/tessel.rs)

Appearance is local wizard state. System follows the theme reported
by egui and uses the dark palette when the desktop theme is unknown.
Windows contrast themes take precedence over the saved preference and
use native colors. The UI uses a compact single-column task at the
minimum size, accessible full-value views, and a fixed action strip
that keeps the next decision in reach. [Accessibility notes](docs/ACCESSIBILITY.md)

## 2026-11-12 — Reopen capture evidence and replay saved approvals

The wizard stores the receipt digest separately from the cleaned capture.
Reopening checks that digest, the capture notes, and the harvested payload
before returning to review. Changing setup detaches the previous capture
and clears its trust and review state. Updating an app requires a fresh
capture, comparison against the previous approval, and a separate saved
profile.

The queue freezes the approved profile, setup, tool folder, and output
folder when an entry is added. It executes serially and records each
result independently. Saved build identity includes the verified receipt
digest, installer SHA-256, and app ID. Installer tests check all three
before opening the dedicated box. Build comparisons retain both installer
SHA-256s and track staged file times before normalization independently
of the content hashes.

Windows Installer packages use a guided Windows Sandbox guest flow.
The clean guest note precedes opening setup, and the guest flow keeps
network and clipboard sharing disabled. Windows Installer can involve
a system service, so the Sandboxie-Plus reusable workflow is scoped to
EXE setup.

## 2026-11-05 — Dedicated Sandboxie-Plus capture for reusable profiles

The explicit Windows x64 capture command provisions an owned fresh
box with the supported Sandboxie-Plus version. It checks setup's
process identity and driver-reported membership before resuming it.
The selected app runs through a verified boxed helper, which waits for
the app's exit. The host observes the supported redirected filesystem
and scoped registry changes, then copies the selected payload.
Unchanged host registry entries are discarded before writing the notes.
The receipt is written outside the box only after cleanup is confirmed.
Profile approval binds that receipt to the same setup, three notes and
harvest. Previous Windows Sandbox profile manifests stay compatible.

## 2026-11-05 — Unattended builds reuse a sealed capture profile

Review and prepare the package once from a completed Windows Sandbox
session. Save with an explicit approval of the prepared package and a
redistribution assertion. The profile records the captured setup and
notes, preserves the exported decisions, and seals the exact package
tree under a separately retained SHA-256. Replay checks the caller's
setup bytes and the approval record before reserving a build folder.
It checks the staged tree again before invoking the existing compiler.
The launcher and settings behavior are the operator's one-time
approval. A matching fingerprint identifies the approved bytes and
does not establish capture coverage or runtime portability.

## 2026-11-05 — Unattended compilation starts from a prepared package

`portableforge compile` takes a trusted Format 3.9 package, an explicit
redistribution assertion, and a trusted local PortableApps.com Installer
3.9.19 installation. This boundary lets the official maker compile a
checked, staged package after the operator prepares a launcher and
reviews the application. It limits the installer settings to offline
bundling and rejects custom installer scripts. The staged maker uses
its local compiler files, and PortableForge pins the four directly
identified executables by hash. The installer and manifest are
published together after the source recompile and embedded icon check.

## 2026-10-25 — Sandboxie installer bundled (SEAN's pick over link/download-in-app)
Offered three ways to get Sandboxie onto a machine with no test
box (download-in-app / bundle / link button), SEAN picked
**bundle**. Basis: Sandboxie-Plus's own `LICENSE.Plus` expressly
permits redistributing "the original unmodified binaries free of
charge", so the bundle is the maker's untouched installer file,
byte for byte, pinned by size + SHA-256 in `tools.rs` (v1.18.8,
24,930,680 B, f27078ff…), fingerprint-verified by the program
before it is ever opened, and staged by the release workflow
(download + hash check) into both assets under `bundled/` with
both license texts and a notice (`third-party/`). PortableForge
stays MIT; no rights over Sandboxie are claimed. Accepted costs
(named when the choice was offered): releases grow ~4 MB →
~29 MB, and the bundled copy ages as Sandboxie releases — the pin
moves in a later PortableForge release when it must.

## 2026-10-25 — Sandboxie quick box (Mode A2), SEAN's pick
Windows Sandbox needs Pro/Enterprise/Education; SEAN's own machine
reports its edition as 'Core' (Windows Home), so the safe test box
can never run there. Offered four ways forward (Sandboxie mode /
productised VirtualBox mode / both / keep current modes), SEAN
picked the Sandboxie mode. Design: Sandboxie stays an external
black box we orchestrate (its own `Start.exe`), never bundled —
the person installs Sandboxie-Plus themselves; PortableForge
detects it in its standard install places. A fresh named box per
session makes the box folder itself the after-state; registry
notes are `reg.exe` exports run inside the box (same three hives
as the agent), so the existing diff/classify/build pipeline is
untouched. Honesty label: fast and works on any edition, but not
a separate computer — deep system parts may not install properly;
the classifier's existing red flags carry that warning. The leak
test still uses the safe/careful ways in this version.

## 2026-10-23 — Name: PortableForge
SEAN chose the name (working title was PAFForge). Repo/workspace renamed
before first code landed, so no legacy name survives in paths.

## 2026-10-23 — First target: Windows Sandbox (Mode A)
Delegated by SEAN ("you decide"). The brief's core promise is "the host
computer is untouched", and only Windows Sandbox delivers it. The
snapshot/diff/classify core is mode-agnostic, so the Home-edition
fallback (Mode C) later reuses the same engine; nothing is wasted.

## 2026-10-23 — Redistribution guardrail: explicit choice, enforced in code
SEAN approved with the choice explicit. In the wizard it is a plain
question ("Are you allowed to share this program?") with a "not sure"
answer that maps to the online-installer distribution. In code,
`Distribution::check_guardrail` makes an unasserted bundle or an
unhashed download unbuildable, and `generate_tree` refuses invalid
metadata.

## 2026-10-23 — GUI: egui (eframe, glow renderer)
SEAN chose egui. Glow is pinned deliberately — a wgpu build with no
backend feature panicked on a real Windows machine in a sibling project.

## 2026-10-23 — Dark UI only
SEAN: "dark UI". PortableForge ships one theme — the black/red family
tokens — with no light theme. Status is never carried by colour alone.

## 2026-10-23 — 100% Rust for authored code
SEAN: "100% Rust code if possible". Everything PortableForge authors is
Rust: host wizard, CLI, core, and the in-sandbox agent. No script files
of any kind; the `.wsb` LogonCommand starts the agent binary directly.
The limits of "possible" are named, not hidden: Windows Sandbox itself,
`reg.exe` (an OS component the agent invokes on Windows), and the
official PortableApps.com Launcher/Installer generators are external
programs we orchestrate — reimplementing them in Rust would break
Format compatibility, which is the entire point of the tool.

## 2026-10-23 — Launcher INI semantics verified against the official template (correction)
Slice 1's generated Launcher `.ini` was written from memory. Before
building on it, the official Application Template 3.5.2 was downloaded
(SHA-256 in `tools-pins/PINS.md`) and its fully-commented example INI
read in full. Three corrections resulted: `[DirectoriesMove]` keys are
single folder names inside `Data` (not sub-paths); `[RegistryKeys]`
keys are bare `.reg` file stems without forced numbering;
`[FileWriteN]` uses the template's path-migration triple
(package dir → base dir → drive, most specific first) with the file
resolved into its moved Data folder. Also recorded: the template's
`[Format] Type=PortableAppsFormat` (3.5) is superseded by the current
Format 3.9 specification (`PortableApps.comFormat`), which we follow.
Lesson banked in the classic form: never generate another project's
file format from memory when its own template can be downloaded.

## 2026-10-23 — Tool pins measured from real downloads
PAL Generator 2.2.2.1 and PA Installer 3.9.22 (latest stable; 3.10 is
beta) plus Template 3.5.2, from the official SourceForge project,
hashed locally — `tools-pins/PINS.md`, pins in core `tools.rs`.
Binaries are a local cache, never committed.

## 2026-10-23 — Fully guided, no-code experience
SEAN: "completely guided … anyone can use it regardless of coding
knowledge". This is a product invariant, enforced mechanically: all
wizard wording lives in core `guide.rs`, primary wording is checked by
a banned-jargon test, the flow is a linear 12 steps with defaults
pre-chosen, and no step can require editing a file by hand.

## 2026-10-23 — PAL manual cross-check: `settings` is a reserved Data folder
The Launcher manual's reference pages (read in full after the template
pass) confirm the template-derived semantics for all four section
types, and add one hard rule the template never mentions: a
`[DirectoriesMove]` key must never be `settings` — `Data\settings`
stores the `.reg` files for `[RegistryKeys]`, and moved data placed
there would be lost. Guard added in the classifier, in specimen
planning, and defensively in generation (a `Settings` folder becomes
`SettingsData`), with a test.
