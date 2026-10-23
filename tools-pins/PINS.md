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
