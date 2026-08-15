# nextar · Windows installer

A professional per-user installer wizard (`nextar-setup.exe`) built from the
`setup/` crate. Double-clicking it runs:

```
Welcome → License → Destination folder (Browse…) → Options → Install → Finished
```

The finished screen offers **Launch nextar-gui** and **Open installation
folder**; the install can be cancelled mid-flight, and the payload is
size-verified before the wizard reports success.

## What it installs

- `nextar.exe` (CLI) and `nextar-gui.exe` (desktop app) into the chosen
  folder (default `%LOCALAPPDATA%\nextar` — **no administrator needed**).
- Per-user **Explorer shell integration** under `HKCU\Software\Classes`:
  - right-click any file/folder → **Compress to .next** / **Compress to .next and email**
  - right-click a folder → **Extract .next… here** (pick an archive, extract into that folder)
  - right-click a `.next` archive → **Open in nextar** / **Extract here** / **Repair with .nvol…**
  - double-click a `.next` archive → opens in nextar-gui (Inspect view)
  - **Send To → Compress to .next** (`%APPDATA%\Microsoft\Windows\SendTo`
    shortcut) — works with multiple selected items
- A proper **Uninstall** entry (Settings → Apps), plus optional Start-Menu
  and desktop shortcuts (the SendTo entry is removed on uninstall).
- A GUI uninstaller (`nextar-setup.exe --uninstall`).

## Upgrades

Re-running `nextar-setup.exe` over an existing install is an **in-place
upgrade**: the wizard detects the installed version (from the per-user
Uninstall key) and relabels its buttons **Upgrade nextar** / **Upgrade**,
overwriting the three binaries and re-applying shortcuts and shell
integration (fixed `.lnk` paths, so no duplicates). Your `settings.json` and
any user archives are left untouched.

## Build

PowerShell (recommended):

```powershell
powershell -ExecutionPolicy Bypass -File installers/windows/build.ps1
```

or manually:

```bash
cargo build --release --bin nextar --bin nextar-gui          # top-level bins
cargo build --release --manifest-path setup/Cargo.toml        # the wizard
# artifacts land in setup/target/release/nextar-setup.exe
```

The wizard **embeds** the two release binaries (`include_bytes!` in
`setup/src/main.rs`), so rebuild them *before* the setup crate. `dist/`
then contains all three exes — run `dist/nextar-setup.exe` to install.

## Code signing

`build.ps1` signs the three `dist/` exes when a certificate is configured
(best effort — signing is skipped otherwise, and an unsigned installer still
works):

```powershell
$env:NEXTAR_SIGN_CERTFILE = 'C:\certs\nextar.pfx'
$env:NEXTAR_SIGN_PASSWORD = '…'
powershell -ExecutionPolicy Bypass -File installers/windows/build.ps1
```

`signtool.exe` (Windows SDK) signs with SHA-256 plus an RFC-3161 timestamp.
For CI, set the same two variables as encrypted secrets and run `build.ps1`
(or a dedicated `signtool` step) before publishing the artifacts.

## MSI (.msi)

The `.exe` wizard is the shipped installer: it is fully per-user (no admin,
no elevation), self-updating in place, and owns its own uninstaller. An
`.msi` package is not produced today — Windows Installer is machine-wide by
default and would need WiX or a comparable authoring tool (an extra
dependency) plus admin elevation, which nextar deliberately avoids. If a
`.msi` is ever required for enterprise deployment, wrap the same payload
with WiX's `perUser` scope; the install actions in `setup/src/main.rs`
remain the single source of truth.

## Before shipping

Replace the sample end-user license text (`LICENSE_TEXT` in
`setup/src/main.rs`) with your real terms before publishing a release.

## Automation flags

| flag | effect |
|---|---|
| `--prefix <dir>` | install into `<dir>` instead of `%LOCALAPPDATA%\nextar` |
| `--quiet` | no confirmation dialogs |
| `--dry-run` | print what would be done without changing anything |

The theme-aware logo, title bar, and window rounding described in the
project README apply to the wizard as well (it reads
`%LOCALAPPDATA%\nextar\settings.json`).

## Verify the shell integration

`scripts/verify-shell.ps1` checks the integration at three levels and exits
nonzero on any failure:

1. **Registry** — every verb key exists with a command pointing at `nextar-gui.exe`.
2. **Explorer** — asks the shell itself (`Shell.Application`) to resolve the
   verbs for a real file, folder, and `.next` archive.
3. **Functional** (`-Run`) — actually invokes the verbs and checks the
   produced artifacts (archive created for file/folder, extracted file for
   the `.next`) plus the full **repair flow**: builds a recovery archive
   with `.nvol`, corrupts it, detects the corruption, invokes the Repair
   verb, and re-verifies the repaired output — both **plaintext and
   password-protected** (the encrypted variant verifies corruption with the
   password, repairs via the shell verb, confirms the encryption survived,
   and byte-compares the re-extracted content).

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/verify-shell.ps1 -Run
```

## Scheduled daily smoke test

`scripts/schedule-shell-test.ps1` registers a Task Scheduler task
(`nextar-shell-test`) that runs the verification every day at 09:00
(interactive — only while you're logged on, so a failure can raise a toast):

```powershell
# register (default: silent registry + Explorer checks, no windows)
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/schedule-shell-test.ps1
# same, but with the full -Run invocation checks (brief progress windows flash)
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/schedule-shell-test.ps1 -Deep
# different time / unregister
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/schedule-shell-test.ps1 -Time 18:30
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/schedule-shell-test.ps1 -Remove
```

`scripts/run-shell-test.ps1` (the task action) reports results three ways:

- **Log** — every run is appended to `%LOCALAPPDATA%\nextar\shell-test.log`.
- **Event log** — a failure writes an Error entry (Event ID 1001) to the
  Application log; the dedicated `nextar` source is used when it exists,
  otherwise the generic `Application` source (no admin needed).
- **Toast** — a failure raises a Windows desktop notification (best effort).

## Troubleshooting: right-click menu missing

After installing, Explorer can serve a **stale context-menu cache**. The
installer now fires `SHChangeNotify(SHCNE_ASSOCCHANGED)` + `ie4uinit -show`
on install/uninstall to invalidate it. If a menu still doesn't appear:

1. Close and reopen any Explorer windows, or
2. Restart Explorer (Task Manager → Windows Explorer → Restart), or
3. Re-run `nextar-setup.exe --quiet` to re-apply registration + cache refresh.
