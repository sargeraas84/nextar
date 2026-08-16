# Provisioning a real EV code-signing certificate

> **Before buying anything: this is an open-source project, so check the
> free path first.** [SignPath Foundation](SIGNING.md#1-signpath-foundation--free-ca-issued-signing-recommended)
> gives approved OSS projects a CA-issued code-signing certificate at **$0**
> — no purchase, no smartcard, no private-key secrets. The whole pipeline is
> already wired for it; you only need to apply and set four secrets. This
> document is the **paid** fallback (for commercial licensing, when the free
> tier's signature quota is insufficient, or if the Foundation application
> is declined).

This is the end-to-end path from "no cert" to "releases show **Verified
publisher: Michael Rieger** everywhere". The self-signed cert in
[`SIGNING.md`](SIGNING.md) silences the warning only on machines that import
it; an **EV (Extended Validation) code-signing certificate** from a public
CA makes every Windows machine trust the signature automatically.

> This doc describes the process and the exact commands. The cert itself
> must be purchased and identity-verified through the CA's portal — that
> part is a human + payment step, not something a script can do.

## 1. What you need before ordering

EV code-signing requires the CA to verify you are a real person (or
business). Have ready:

- **Legal name** exactly as it will appear (e.g. `Michael Rieger`)
- **Organization** (optional for individual certs; required for OV) + **address**
- **Phone + email** for the call-back verification
- **A hardware token or cloud keystore** the CA supports (DigiCert:
  Signing Manager / Key Vault; Sectigo: USB token or online key)
- **~$200–600/yr** (EV individual certs are typically at the lower end,
  OV/EV org certs higher)

Timeline: EV verification usually takes **1–3 business days** after order
(the CA calls you). Plan releases around it.

## 2. Order (provider-specific)

### DigiCert (recommended — cleanest GitHub Actions path)

1. Go to <https://www.digicert.com/signing/code-signing> → **EV Code
   Signing Certificate**.
2. During checkout choose **Signing Manager / Key Vault** as the key
   storage (cloud-hosted key, exportable PFX) — this is what makes the
   GitHub secret path work. A USB token also works but you must be at the
   machine to sign, which defeats CI signing.
3. Complete identity verification (call-back + documents).
4. In **DigiCert Signing Manager** → your certificate → **Key Vault**,
   create a vault and generate the cert into it.

### Sectigo

1. <https://sectigo.com/ssl-certificates-tls/code-signing> → **EV Code
   Signing**.
2. Choose the delivery that gives you a downloadable `.pfx`/`.jks`
   (software-based key) so you can store it as a GitHub secret. USB token
   delivery also exists but blocks CI signing.
3. Identity verification, then download the PFX from your account.

### Azure Trusted Signing (no PFX, different workflow)

Microsoft's managed signing service keeps the key in their HSM — there is
no PFX. If you pick this, skip steps 3–4 below and use the
`azure/trusted-signing-action` snippet documented in [`SIGNING.md`](SIGNING.md)
instead.

## 3. Export the PFX (once, on your machine)

After the CA issues the cert, export it as a **password-protected PFX**
containing the private key:

```powershell
# DigiCert Signing Manager → Key Vault → "Export" gives you the pfx directly.
# Sectigo: download from your account.
# If you get a .cer/.key pair instead, convert with openssl:
openssl pkcs12 -export -out code-signing.pfx -inkey private.key -in cert.cer
```

Verify the export contains the private key (it must say `hasPrivateKey`):

```powershell
$c = New-Object System.Security.Cryptography.X509Certificates.X509Certificate2
$c.Import('C:\path\to\code-signing.pfx', 'the-pfx-password', 'DefaultKeySet')
$c.Subject; $c.HasPrivateKey
# CN=Michael Rieger, O=...   True
```

## 4. Store as GitHub secrets

The release workflow already detects these two secrets and signs + verifies
automatically (see the `Code-sign installer and binaries` and `Verify
Authenticode signatures` steps in `.github/workflows/release.yml`):

```powershell
$pfxB64 = [Convert]::ToBase64String([IO.File]::ReadAllBytes('C:\path\to\code-signing.pfx'))
gh secret set CODE_SIGN_PFX -b $pfxB64
gh secret set CODE_SIGN_PFX_PASS -b 'the-pfx-password'
```

Secrets are repo-scoped by default. They are masked in logs and only
referenced inside the signing/verify steps.

## 5. Cut a release and verify

```bash
git tag v0.3.0 && git push origin v0.3.0
```

Watch the run: the Windows job imports the cert, signs the three exes,
verifies each signer thumbprint matches, runs the install E2E, and ships.
Then verify the published artifact on a **clean machine** (or after
`Clear-RecycleBin`-style clean profile — anything without the self-signed
cert in its store):

```powershell
(Get-AuthenticodeSignature nextar-setup.exe).Status   # Valid
(Get-AuthenticodeSignature nextar-setup.exe).SignerCertificate.Subject
# CN=Michael Rieger, O=... C=...
```

With an EV cert this shows `Valid` **and** SmartScreen shows
"Verified publisher" with your name — no cert import needed on the target
machine.

## 6. Rotating / renewing

- **Renewal** keeps the same subject — export the new PFX and re-run the
  two `gh secret set` commands.
- **Revocation** (key compromised): revoke in the CA portal immediately,
  then re-run `gh secret set` with the new PFX and cut a fresh release.
- Never commit the PFX or password (`.gitignore` already blocks `*.pfx`).

## FAQ

**Does EV remove the SmartScreen warning entirely?**
For downloads, SmartScreen shows "Verified publisher" instead of "Unknown
publisher" once Microsoft's reputation service has seen the cert signed
enough files (a few downloads usually suffice). It can still show an
informational "file is uncommon" notice on brand-new files — that fades as
the cert accumulates reputation.

**OV vs EV?**
OV (Organization Validation) also stops the "unknown publisher" warning but
does not give the immediate reputation boost / stronger identity bar that
EV does. For a public archiver, EV is the better investment.

**Can I test before buying?**
Yes — the whole pipeline is already exercised with the self-signed cert
(commit `0aa1715` signed + verified in CI). Buying the EV cert only swaps
the PFX in the two secrets; no code or workflow changes needed.
