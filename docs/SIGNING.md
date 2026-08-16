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
A real CA cert makes the release show **"Verified publisher: <you>"** in
SmartScreen/Explorer everywhere, no cert distribution needed.

### Which provider

| Provider | Private key model | Fits the PFX-secret path below? |
|----------|-------------------|----------------------------------|
| **DigiCert** (Signing Manager / Key Vault) | Cloud key, exportable PFX | ✅ yes |
| **Sectigo** (Code Signing Certificates) | PFX / key export | ✅ yes |
| **Azure Trusted Signing** | Key never leaves Microsoft | ❌ no PFX — needs the `azure/trusted-signing-action` instead |

### Path A — DigiCert or Sectigo (PFX secrets)

1. Purchase an **OV or EV code-signing certificate** (EV shows a stronger
   identity bar; either works for Authenticode).
2. In the provider's portal, request the cert with the subject set to your
   legal name or company (e.g. `CN=Michael Rieger, O=Michael Rieger`).
3. Export the certificate + private key as a password-protected `.pfx`
   (DigiCert Signing Manager → Key Vault → export; Sectigo → My Account →
   download PFX).
4. Store it as GitHub repo secrets:

   ```powershell
   $pfxB64 = [Convert]::ToBase64String([IO.File]::ReadAllBytes('C:\path\to\code-signing.pfx'))
   gh secret set CODE_SIGN_PFX -b $pfxB64
   gh secret set CODE_SIGN_PFX_PASS -b 'the-pfx-password'
   ```

5. Tag a release (`git tag v0.3.0 && git push origin v0.3.0`). The `release`
   workflow's Windows job detects both secrets, imports the cert, signs
   `nextar.exe`, `nextar-gui.exe`, and `nextar-setup.exe` (SHA-256 +
   RFC3161 timestamp), **verifies each signature's signer thumbprint matches
   the imported cert**, then runs the full install E2E and ships the signed
   artifacts. If signing or verification fails, the job fails loudly and no
   release is published.

   Secrets exposure: `CODE_SIGN_PFX` and `CODE_SIGN_PFX_PASS` are set as
   job-level `env` and consumed only inside the signing/verify steps. They
   are masked in logs (GitHub redacts secret values).

6. The first signed release proves it: `Get-AuthenticodeSignature` on the
   downloaded `nextar-setup.exe` shows `Status: Valid` with your CA's chain
   (and SmartScreen's "unknown publisher" warning is gone on all machines).

### Path B — Azure Trusted Signing (no PFX)

Azure Trusted Signing keeps the key in Microsoft's HSM — there is no PFX to
store, and the release workflow's PFX step cannot be used. Wire the official
action instead, before the existing signature-verify gate:

```yaml
- name: Sign with Azure Trusted Signing
  uses: azure/trusted-signing-action@v0
  with:
    endpoint: https://eus.codesigning.azure.net   # your region
    trusted-signing-account-name: nextar-codesign
    certificate-profile-name: nextar-profile
    files: |
      dist/nextar.exe
      dist/nextar-gui.exe
      dist/nextar-setup.exe
  env:
    AZURE_CLIENT_ID: ${{ secrets.AZURE_CLIENT_ID }}
    AZURE_TENANT_ID: ${{ secrets.AZURE_TENANT_ID }}
    AZURE_CLIENT_SECRET: ${{ secrets.AZURE_CLIENT_SECRET }}
```

Create the account + identity in the Azure portal, add the three secrets
above, and grant the identity the **Trusted Signing Certificate Profile
Signer** role.

### Updating an existing release's checksums (backfill)

The release body carries `**Checksums (SHA-256)**` lines that the landing
page's stable card reads. If a release was cut before the checksum step
existed, backfill them manually:

```bash
sha256sum nextar-setup.exe nextar-0.2.0-macos.dmg
# append the `**Checksums (SHA-256)**` block to the release notes via
# `gh release edit <tag> --notes-file notes.md`
```

## Notes

- Signing does not change functionality — it only adds the Authenticode
  signature block so Windows can attribute the publisher.
- `sign.ps1` (local) and the release workflow (CI) use the same signature
  parameters (`/fd SHA256 /tr http://timestamp.digicert.com /td SHA256`) so
  local and CI builds verify identically.
- `package.sh` signs **locally only**. In CI (`CI=true`) it skips signing —
  the release workflow signs with the real cert via secrets, and the nightly
  intentionally does **not** sign (rolling builds). This also avoids the
  `sign.ps1` self-signed-cert/timestamp calls, which hang on GitHub Windows
  runners (see commit `12c0489`).
- The signature-verify gate accepts `UnknownError` from
  `Get-AuthenticodeSignature` when the signer thumbprint matches — for a
  self-signed cert that is a trust-chain status, not a broken signature. A
  real CA cert reports `Valid` on any machine.
