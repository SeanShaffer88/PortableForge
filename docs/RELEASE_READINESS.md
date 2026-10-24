# Release-readiness checklist

Nothing ships as a PortableForge release until every box here is
checked with evidence — a green tick means "seen", never "assumed".

## Code gate

- [ ] `cargo fmt --all --check` clean
- [ ] `cargo clippy --locked --all-targets -- -D warnings` clean (host)
- [ ] `cargo clippy --target x86_64-pc-windows-msvc --locked --all-targets -- -D warnings` clean
- [ ] `cargo test --locked` — full suite, no filters, all passing
- [ ] CI run on the exact release commit is green on windows-latest
- [ ] Every file on `main` blob-verified against the local tree

## On-machine evidence

- [ ] `docs/ON_MACHINE_RUNBOOK.md` passed on SEAN's machine for this
      version, with the session folder kept as evidence
- [ ] The official PortableApps.com makers have run against a built
      package on a real machine at least once for this version line
- [ ] Anything not run is labelled Unverified in the release notes —
      in those words

## Package and paperwork

- [ ] Version numbers agree everywhere: workspace crates, `--version`
      output, the wizard's in-window version line, CHANGELOG heading
- [ ] CHANGELOG entry written in plain language, including what is
      *not* verified
- [ ] README opens with `## TL;DR`; every claim in it traces to a
      test, a CI run, or an on-machine result (the final audit)
- [ ] README screenshots are real captures from SEAN's machine,
      refreshed if the window changed materially
- [ ] Repo About description and topics match what this release
      actually contains

## The release itself

- [ ] Tag `vX.Y.Z` on the exact gated commit
- [ ] Release zip contains both programs, README, LICENSE, and the
      runbook
- [ ] SHA-256 of every asset printed in the release notes and
      verified by downloading the assets back
- [ ] The build is unsigned, and the release notes say so plainly,
      with the SmartScreen note a first-time user will need
