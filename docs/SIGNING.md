# Code signing

nextar ships with an optional code-signing setup so Windows stops warning
about "unknown publisher" on machines that trust the signing certificate.

There are two tiers:

| Tier | Trusted where | How |
|------|---------------|-----|
| Self-signed "Michael Rieger" cert | machines that import the cert | `scripts/sign.ps1` + `scripts/export-signing-cert.ps1` |
| Real CA cert (recommended for public releases) | everywhere | store as a GitHub secret, release workflow signs automatically |

## 1. Self-signed cert (local builds, this machine)

`scripts/sign.ps1` creates (once) a `CodeSigningCert` named **Michael Rieger**
in the current user's store, trusts it in the per-user Trusted Root +
TrustedPeople stores, and signs every exe:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/sign.ps1
```

Signed targets (when they exist): `target/release/nextar.exe`,
`target/release/nextar-gui.exe`, `setup/target/release/nextar-setup.exe`,
and `dist/nextar-setup.exe`. Signature is SHA-256 + RFC3161 timestamp
(DigiCert timestamp server).

`scripts/package.sh` runs this automatically (best effort — it skips with a
notice if signtool or the cert is missing, so the build still succeeds).

Verify signatures:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/check-signatures.ps1
```

### Export the cert for other machines

```powershell
powershell -ExecutionPolicy Bypass -File scripts/export-signing-cert.ps1
# or with a set password for CI:
powershell -ExecutionPolicy Bypass -File scripts/export-signing-cert.ps1 -PfxPassword 'change-me'
```

Writes:
- `dist/certs/nextar-rieger.pfx` — private key (password-protected). **Keep
  it secret** — anyone with it can sign as you.
- `dist/certs/nextar-rieger.cer` — public cert, safe to share.

To make Explorer trust builds on another machine, import the public cert:

```powershell
Import-Certificate -FilePath nextar-rieger.cer -CertStoreLocation Cert:\CurrentUser\Root
```

## 2. Real CA cert (CI / public releases)

For releases that other people download, a real code-signing certificate is
required — self-signed certs are only trusted where the cert is installed.
Obtain one (DigiCert, Sectigo, Azure Code Signing, ...), export it as a
`.pfx`, then store it as GitHub repo secrets:

```powershell
$pfxB64 = [Convert]::ToBase64String([IO.File]::ReadAllBytes('dist\certs\nextar-rieger.pfx'))
gh secret set CODE_SIGN_PFX -b $pfxB64
gh secret set CODE_SIGN_PFX_PASS -b 'the-pfx-password'
```

When both secrets exist, the `release` workflow's Windows job imports the
cert, signs `nextar.exe`, `nextar-gui.exe`, and `nextar-setup.exe` with
SHA-256 + RFC3161 timestamp, and ships the signed artifacts in the release.
Without the secrets, releases still build — they just ship unsigned
(`if: secrets.CODE_SIGN_PFX != ''`).

## Notes

- Signing does not change functionality — it only adds the Authenticode
  signature block so Windows can attribute the publisher.
- `sign.ps1` and the release workflow use the same signature parameters
  (`/fd SHA256 /tr http://timestamp.digicert.com /td SHA256`) so local and
  CI builds verify identically.
- The nightly workflow intentionally does **not** sign (rolling builds);
  sign only the versioned releases or local distro copies.
