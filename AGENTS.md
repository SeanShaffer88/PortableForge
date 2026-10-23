# AGENTS.md — PortableForge

The Universal Windows Desktop Master Prompt applies in full:
`~/workspace/your_files/windows-desktop-master-prompt.md`

## Project in one line

A guided Windows program (Rust + egui, dark black/red UI) that automates
making PortableApps.com Format packages: pre-scan, install, run, close,
post-scan inside a disposable Windows Sandbox, then classify, generate,
build, and leak-test — so the host machine is untouched.

## Hard rules

- **Guided, plain language, always.** The wizard is the product. Primary
  wording lives in `crates/portableforge-core/src/guide.rs` and is
  enforced by the jargon-guard test (`BANNED_IN_PRIMARY`). Never put
  jargon into primary UI strings; technical detail goes behind
  "Show details". No step may require editing code or config files.
- **100% Rust for authored code.** No PowerShell, batch, Python, or other
  script files anywhere in the product or build. The `.wsb` logon command
  launches the Rust agent directly. External black boxes (the official
  PortableApps.com Launcher/Installer generators, Windows Sandbox,
  `reg.exe`) are invoked, never reimplemented, pinned and SHA-256
  verified where they are files we ship.
- **Dark UI only**, family tokens in `guide::theme`
  (#000000/#101010 bg, #242424 hairlines, #6B0700 primary red,
  #E5484D accent text, #F5F5F5/#A8A8A8/#6E6E6E text). No light theme.
- **Sandbox honesty.** Mode A (Windows Sandbox) and Mode B (user VM) are
  isolated. Mode C (host snapshot) is NOT isolated and must always be
  labelled "watched and put back, not sealed off" — in UI, logs, reports.
- **Redistribution guardrail is code, not copy.** Bundled-files packages
  require the user's explicit sharing assertion; "not sure" maps to the
  online-installer distribution with URL + SHA-256. See
  `paf::Distribution::check_guardrail`. Never weaken it silently.
- **Noise is flagged, never dropped.** Snapshot diffs keep noise entries
  marked; classification shows them collapsed.
- **Never claim Windows runtime is verified** until run on SEAN's
  Windows machine and confirmed. Label Verified / Partially Verified /
  Unverified.
- Public documentation must not mention or compare competitor software.

## Architecture

- All decision logic lives in `portableforge-core` and is testable
  off-Windows: snapshot, `.reg` parsing, diff, noise, classify, PAF
  generation/validation, `.wsb` generation, session model, guide wording.
- The GUI renders state and collects intent only; the agent binary is a
  thin executor of core functions inside the sandbox.
- Sessions are JSON folders on disk; interrupted runs resume.

## The gate (run in full before publishing)

```sh
export PATH="$HOME/.cargo/bin:$PATH"
cargo fmt --all
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo check --target x86_64-pc-windows-msvc --all-targets   # when the target is installed
```

Full suite, never a filtered subset. CI mirrors this on windows-latest.

## Documentation

- README opens with `## TL;DR`; screenshots only from real runs on
  SEAN's machine (capture list: `docs/screenshots/CAPTURE.md`).
- Decisions in `DECISIONS.md`; user-facing changes in `CHANGELOG.md`.

## License

MIT — see `LICENSE`. Copyright 2026 Sean Shaffer.
