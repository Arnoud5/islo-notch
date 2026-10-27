# Islo to-do

## Notifications without Developer Mode

Islo's notifications package (`package\`) is unsigned, and Windows only registers
unsigned packages with Developer Mode on. That's why Settings → Notifications →
**Set up** turns Developer Mode on first (one admin prompt). Everything else in
Islo works without it.

To drop that step, the package has to be **signed** with a certificate Windows
trusts:

| Option | Admin prompt for users | Cost | Notes |
|---|---|---|---|
| Keep Developer Mode (now) | Once | Free | Machine-wide setting; some users won't want it on |
| Self-signed certificate | Once, to trust the certificate | Free | Islo installs the cert as trusted, then registers a signed package. Narrower than Developer Mode, still a prompt |
| Real code-signing certificate | None | Microsoft Trusted Signing (Azure) about $10/month; classic certificates about $100–400/year | Needs an identity check, and Trusted Signing is only open to individuals in some countries. Also signs Islo.exe, so SmartScreen stops warning about an unknown publisher |
| Microsoft Store | None | One-time developer account fee | No longer a portable exe; updates go through the Store instead of GitHub |

The real certificate is the only option that removes every prompt and keeps Islo
portable with GitHub updates. Until then, Developer Mode is the simplest choice.
