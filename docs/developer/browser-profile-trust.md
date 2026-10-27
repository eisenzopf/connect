# Isolated browser TLS profiles

G6-051's actual browser lane uses normal certificate verification with a fresh
private profile. `scripts/video/browser_profile.py` supports the recorded Chrome
155.0.8059.85 database schema and Firefox's bundled NSS 3.130. It refuses an
existing profile, including an empty directory or symlink. Unknown Chrome/NSS
versions refuse rather than guessing a database or ABI. The Firefox app stays
signed and unchanged; its public NSS API imports a website-only CA into the new
profile. Chrome's profile database stores the CA using its versioned certificate
metadata. Neither path modifies the system Keychain or another browser profile.

Create a new ignored directory with mode 0700 for the synthetic certificates,
keys, profiles and raw logs. Create a CA and two localhost-only leaves under it:
one leaf signed by that CA and a second leaf signed by a different CA. The leaf
SAN must contain `DNS:localhost` and exclude `IP:127.0.0.1`, so the same real
listener also supplies the hostname-negative control. The leaves need server
authentication usage; the CA needs CA basic constraints. Export the first CA as
DER. Keep private keys and certificate-generation logs under `.local/`.

Run the following with absolute paths, substituting that fixture's paths:

```sh
python3 scripts/video/check-browser-profile.py chrome \
  '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome' \
  .local/fixture/root.der .local/fixture/localhost.pem .local/fixture/localhost.key \
  .local/fixture/foreign.pem .local/fixture/foreign.key .local/fixture/chrome-trusted
```

Repeat with a separate new output and `--without-root`. Repeat both cases with
`firefox` and its designated private executable. Every run creates its own new
profile and loopback HTTP bootstrap plus two real TLS listeners. The page checks
HTTPS and WSS for the correct certificate, foreign CA and wrong hostname. Only
the trusted correct-name requests may reach application handling. Without the
root, all six connections must refuse. There are no certificate exception flags,
HTTPS-error options or global trust changes. Each original browser process group receives termination; its original process
and all listeners are joined before the result is written.

For a real signed-host fixture, generate a fresh certificate cohort and seed each
new owned profile before launching it. Pass the proper CA separately to the
native client; serve the localhost leaf on the host. Preserve the browser's
independent credentials, explicit accepted Session and original receive
selection. The TLS check alone does not qualify canonical origin policy,
signature/enrollment, media, rendering, fallback or any Gate 6 acceptance item.

The recorded local observations and earlier fixture failures are in
`docs/evidence/gate-6/browser-profile-trust.json`. The importer uses Chromium's
[155 certificate metadata](https://chromium.googlesource.com/chromium/src/+/refs/tags/155.0.8059.85/components/server_certificate_database/server_certificate_database.proto)
and the NSS public [certificate API](https://github.com/mozilla/nss/blob/NSS_3_130_RTM/lib/certdb/cert.h).
Mozilla documents `C,,` as
[website CA trust](https://firefox-source-docs.mozilla.org/security/nss/runbooks/rootstore.html).
