# SignPath Foundation application — paste-ready content

Everything the [signpath.org](https://signpath.org) **Apply for Free Code
Signing** form asks for, pre-filled. Copy the values below into the form and
submit. If a field is not asked for, skip it.

## Project identity

| Field | Value |
|-------|-------|
| Project name | **nextar** |
| Contact name | Michael Rieger |
| Contact email | michaelrieger@laposte.net |
| Repository URL | https://github.com/sargeraas84/nextar |
| Download page URL | https://sargeraas84.github.io/nextar/ (GitHub Releases: https://github.com/sargeraas84/nextar/releases) |
| License | MIT (OSI-approved; `LICENSE` file in the repo root) |

## Description (paste into the description field)

> **nextar** is a next-generation file archiver for Windows and macOS,
> written from scratch in Rust to take on WinRAR and 7-Zip. It combines the
> best modern open-source technologies:
>
> - **Fast + max compression** — Zstandard for the default "fast" tier and
>   LZMA2 for maximum ratio, in a fully multi-threaded pipeline that reads,
>   compresses, encrypts and writes across every CPU core.
> - **Military-grade security** — Argon2id key derivation with
>   XChaCha20-Poly1305 authenticated encryption, so archives are private
>   *and* tamper-evident.
> - **Self-healing archives** — Reed-Solomon recovery volumes (`.nvol`) let
>   an archive repair itself when bits flip or a download is incomplete.
> - **Instant indexing** — metadata lives in a smart footer, so listing and
>   extracting a single file never means scanning a multi-gigabyte archive
>   sequentially.
>
> The project ships a polished desktop GUI (create / extract / inspect /
> repair / settings with drag-and-drop), a CLI, a Windows installer with
> Explorer shell integration, and a macOS .dmg. It is actively maintained
> with nightly builds, automated install/upgrade/uninstall E2E tests on
> every release, and a public CI pipeline. It is open source under MIT.

## Eligibility (per their terms — we already comply)

- **OSS license:** MIT — `LICENSE` in the repo root, `license = "MIT"` in
  `Cargo.toml`.
- **No proprietary code:** everything is authored in the repo (Rust backend,
  egui GUI, installer wizard, landing page); third-party crates are all
  OSI-approved.
- **Maintained:** active development with nightly builds and scheduled CI.
- **Released:** v0.3.0 is live on GitHub Releases with signed (self-signed
  cert) Windows installer + macOS dmg.
- **Documented:** functionality is described on the landing page
  (https://sargeraas84.github.io/nextar/).

## After approval — what you must add to the project

Once approved, the Foundation requires (per their conditions for the
website/repository) a **Code signing policy** on the project home page.
Ready-to-paste section:

```markdown
### Code signing policy

Free code signing provided by SignPath.io, certificate by SignPath
Foundation.

- Committers and reviewers: [Members](https://github.com/sargeraas84/nextar/settings/access)
- Approvers: [Owners](https://github.com/sargeraas84/nextar/settings/access)
- Privacy: this program will not transfer any information to other networked
  systems unless specifically requested by the user or the person installing
  or operating it.

This program is free software: you can redistribute it and/or modify it
under the terms of the MIT License.
```

Then post the four values from the SignPath dashboard here (org ID, project
slug, signing policy slug, API token) so the `SIGNPATH_*` secrets can be set
and the next release signs automatically.
