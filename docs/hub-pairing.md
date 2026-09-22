# Connect your own hosts

The default Hub workflow uses an authenticator to pair your own devices.
The host verifies the code and remembers the client key. Later connections use
end-to-end encryption without another code. Guest sharing is outside this flow.

```text
Browser → installed local client ⇄ Hub ⇄ outbound host connector → viewer-api
                         └──── end-to-end encryption ────┘
```

The Hub carries public registration information and encrypted traffic. It never
receives the authenticator seed or raw code. Serve the client UI from your local
installation; entering codes into JavaScript supplied by the VPS would trust it
with those codes.

## Build and run

From this checkout:

```sh
cargo build -p tokn-session-hub -p tokn-viewer-api
pnpm --dir apps/viewer install --frozen-lockfile
pnpm --dir apps/viewer build
```

On the VPS, start the Hub behind your HTTPS reverse proxy:

```sh
tokn-session-hub serve --public-url https://hub.example.com
```

The listener defaults to `127.0.0.1:5559`. Proxy WebSocket upgrades on
`/hub/v1/tunnel` and `/hub/v1/secure/*`. Passkey setup is optional administration
for this workflow; paired hosts register without a separate approval ceremony.
Use [Hub administration](hub.md) to inspect or remove registered hosts.

On each host, run the API and connector in separate terminals:

```sh
tokn-viewer-api --api-only
tokn-session-hub connect --hub https://hub.example.com --name "Workstation"
```

On first setup, the connector generates its UUID, dedicated keys, and
authenticator seed. Scan the terminal QR with your authenticator, then copy the
printed host ID. The manual setup key also works with a time-based entry using
SHA1, six digits, and a 30-second period. Setup secrets print only to an
interactive terminal. To display them later over a trusted terminal or SSH:

```sh
tokn-session-hub authenticator
```

On your client machine:

```sh
tokn-session-hub client --hub https://hub.example.com --web-root ./apps/viewer/dist
```

Open the printed local link. Under **Add a host**, enter the host ID and current
authenticator code. After pairing, the viewer opens automatically. **Change
host** returns to your saved hosts and allows adding another. Switching cancels
old requests; an old tab cannot silently send input to the newly selected host.

After first setup, the Hub address and host configuration are saved:

```sh
tokn-session-hub connect
tokn-session-hub client --web-root ./apps/viewer/dist
```

The client reopens its last selected host without another OTP. Use
`client --host HOST_UUID` to select another saved host. The viewer API still
needs to be running on the host. In these examples installed binaries are on
`PATH`; from a checkout, use `./target/debug/tokn-session-hub` and
`./target/debug/tokn-viewer-api` instead.

For a local development Hub use `--hub http://localhost:5559 --insecure-loopback`
on first setup of both connector and client. That explicit development setting
is remembered and never permits HTTP to a remote host.

## Configuration and device access

State defaults to `~/.tokn/hub`; use `--state-dir PATH` consistently to keep a
different installation or test separate. Host files include `host.json`,
`host-enrollment.key`, `host-noise.key`, and `host-access.json`. Client files
include `client.json`, `client-noise.key`, and `client-hosts.json`. Back up the
whole directory using your device's storage controls. Existing corrupt, missing,
or insecure identity/trust files fail closed instead of silently resetting trust.
On Unix these files must have mode 0600 and belong to the current user.

Every paired client is one of your own trusted devices and can view the host's
sessions. Agent input is disabled until the host explicitly enables it:

```sh
tokn-session-hub connect --allow-control
tokn-session-hub connect --allow-control=false
```

That setting persists. Use `--viewer-url` for a different numeric loopback API
origin, and provide `TOKN_VIEWER_TOKEN` if the API requires it; the token is not
saved in configuration or transmitted to the Hub.

List and remove paired devices locally on the host:

```sh
tokn-session-hub devices
tokn-session-hub forget-device --public-key CLIENT_PUBLIC_KEY
```

Removal denies new requests and closes streams when the host next checks its
local state (every second). It cannot recall already delivered content or undo
accepted agent input. Possession of the authenticator still allows pairing again.

## One authenticator for several hosts

Each host keeps distinct UUID and asymmetric keys. To share only the
authenticator seed, export it explicitly from one host and transfer the file
through a trusted channel:

```sh
tokn-session-hub authenticator --export-file ./authenticator.secret
# On a new host, after transferring the private file:
tokn-session-hub connect --hub https://hub.example.com \
  --totp-secret-file ./authenticator.secret
```

Exports create a new mode-0600 file and refuse overwrites. Imports require a
private Base32 file and work only before the authenticator is configured.
Alternatively, run `authenticator --import-file FILE` before the first `connect`.
Synchronization and safe handling of that file remain the user's responsibility;
the Hub does not synchronize seeds or trust records.

Each host consumes a successful code once per time step. A shared code can be
used independently on another host. Every seed-holding host can generate the
group's codes, so compromising one threatens enrollment across that group,
including impersonation during first pairing. Previously saved host keys remain
pinned and cannot be silently replaced. Keep endpoint clocks correct; a code
near its rollover may require explicitly retrying with a fresh one.

## Protocol and limits

Hosts sign a challenge binding their UUID, enrollment public key, display name,
and control setting. Registration establishes routing and key possession, not
client authorization. UUID ownership and Hub removals persist; a removed UUID
cannot silently register again. New registrations are limited to eight per
minute, 64 active hosts, and a bounded revocation ledger. Administrators can
reclaim active capacity by removing unused hosts. Reconnects bypass the new-host
rate limit. Public registration remains susceptible to availability abuse;
restrict admission at the reverse proxy for an exposed deployment.

Pairing uses RustCrypto `spake2` 0.4 with separate HKDF/HMAC confirmations binding
the target ID, both Noise keys, the time step, and the complete handshake.
The host persists five attempts per five minutes across reconnects/restarts,
consumes successful time steps atomically, and allows up to 64 paired devices.
The client saves its pin only after authenticated acknowledgment. Normal access
uses fresh `Noise_IK_25519_ChaChaPoly_BLAKE2s` channels and the host's local
authorized-device list. There is no plaintext fallback or automatic request retry.

The SPAKE2 dependency and this protocol composition have not had an independent
cryptographic audit. This application protocol is not RFC 9382 wire-compatible.
Treat authenticator pairing as experimental pending that review.

The older [owner-signed grant workflow](hub-e2ee.md) and explicit
`connect --trusted-hub` compatibility mode remain available, but are not part of
normal pairing setup. Passkeys govern Hub administration, not paired-device
access to encrypted host content.
