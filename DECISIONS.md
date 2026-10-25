# Decisions — PortableForge

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
