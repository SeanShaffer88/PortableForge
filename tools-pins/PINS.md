# Tool pins — measured, not remembered

Downloaded from the official PortableApps.com SourceForge project on
2026-10-23 and hashed locally. These values are the pins in
`crates/portableforge-core/src/tools.rs`. The binaries themselves are
**not committed** (see `.gitignore`); this folder is a local cache.

| Tool | Version | File | Bytes | SHA-256 |
|---|---|---|---|---|
| PortableApps.com Launcher | 2.2.2.1 | `PortableApps.comLauncher_2.2.2.1.paf.exe` | 2,719,624 | `f12bf700c931bfbe21ea6751dececc681a4d2cdb9a0940bc100e3ff38edf54e0` |
| PortableApps.com Installer | 3.9.22 | `PortableApps.comInstaller_3.9.22.paf.exe` | 6,425,848 | `689456f1fb1320c45e82a077afe6b87fb388fd3f75c41d9ee9ce8d45f3966b67` |
| Application Template | 3.5.2 | `PortableApps.com_Application_Template_3.5.2.zip` | 341,702 | `f432bf127820aa36d160c5e83e75df2d87ea3bda25bfc3c36da18f94c76ffa00` |

Template note: the template's example INIs (3.5.2) were used to verify
Launcher `.ini` semantics. Where the template and the current Format
specification (3.9) differ — e.g. `[Format] Type=` — the 3.9
specification wins for `appinfo.ini`.

## Bundled, unmodified: the Sandboxie installer (Mode A2's way in)

Since v0.2.1 (SEAN's pick), PortableForge **bundles** the official
Sandboxie-Plus installer and offers to open it on computers that
have no test box yet. Redistribution basis, from the maker's own
`LICENSE.Plus`: "It is permitted to redistribute the original
unmodified binaries free of charge." The bundle is exactly that —
the original file, byte for byte, free of charge, with both
license texts and a notice (`third-party/`). The program checks
the fingerprint below before the installer is ever opened.

| Tool | Version | File | Bytes | SHA-256 |
|---|---|---|---|---|
| Sandboxie-Plus installer (x64) | 1.18.8 | `Sandboxie-Plus-x64-v1.18.8.exe` | 24,930,680 | `f27078ff74a07906fd51476079e1a7a8f5e20c8bb75a9d2ae86169d47179ae76` |

Measured 2026-10-25 from the official download
(`https://github.com/sandboxie-plus/Sandboxie/releases/download/v1.18.8/Sandboxie-Plus-x64-v1.18.8.exe`).
The binary is never committed: the release workflow downloads it
and verifies the fingerprint before staging it into both assets.

Once installed, Sandboxie itself is detected (never the bundled
copy) in its standard places and driven through its own command
line only:

| What | Where it is looked for |
|---|---|
| `Start.exe` (Sandboxie-Plus install) | `C:\Program Files\Sandboxie-Plus\Start.exe` |
| `Start.exe` (Sandboxie Classic install) | `C:\Program Files\Sandboxie\Start.exe` |

`Start.exe /box:<name> <program>`, the same form for boxed
`reg.exe` exports, and `/terminate` to end a box's programs. A
custom Sandboxie `FileRootPath` (boxes living somewhere other
than the default `C:\Sandbox\<user>\<box>`) is a known limitation:
captures come back empty with a warning that says so.
