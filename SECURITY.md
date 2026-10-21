# Security policy

## Reporting a vulnerability

Please report security problems **privately**, not in a public issue.

Use GitHub's private vulnerability reporting: open the repository's **Security** tab and choose **Report a vulnerability**. Include the version of Kin on the PC and on the phone, what you did, and what happened. Please do not post pairing codes, keys or the contents of someone's notifications, clipboard or files in a report.

I will answer as soon as I can and will credit you in the release notes if you want. Kin Relay is a small project maintained by one person, so there are no guaranteed response times and no bug bounty.

## Supported versions

Only the **latest release** is supported. Kin Relay is early alpha software; fixes go into a new release, not into old ones.

## How Kin is meant to be secure

This is a short description so you can judge the design and look for holes. The full protocol is in [docs/protocol.md](docs/protocol.md).

- **Local network only.** The phone connects to the PC over your own network. There is no server, no account and no telemetry; nothing leaves your network.
- **Mutual TLS 1.3 with pinned keys.** Every install makes its own P-256 key and a self-signed certificate. Both sides must present a certificate during the handshake, and each side only trusts the **public key** (SPKI hash) of the device it was paired with. There is no certificate authority. The TLS 1.3 signature of a client is always verified, so a copied public certificate is useless without the private key.
- **Pairing with a one-time token.** The PC shows a QR code (or a link) that contains its address, its key fingerprint and a one-time token: **16 random bytes, valid for 5 minutes, and dead after 5 wrong tries**. A pairing link that arrives from outside the app has to be confirmed by the person holding the phone.
- **Private keys at rest.** On Windows the key file is encrypted with DPAPI (tied to your Windows user). On Android the key is encrypted with an AES key held in the Android Keystore.
- **Sensitive content.** Clipboard text marked as sensitive (for example copied from a password manager) is handled specially: it is not stored in the history and is not queued for later, and on the PC it is kept out of the Windows clipboard history and cloud clipboard. Logs never contain the text of what was sent.

## What is exposed on your network

- The Windows app listens on **TCP port 47821** (or a random port if that is taken) on **all interfaces**.
- The installer adds a Windows Firewall rule named "Kin Relay" that allows inbound connections only from the **local subnet**, on **both the Private and the Public network profiles** (Windows marks new networks as Public by default). If your phone is on a different subnet than the PC, you have to edit the rule's scope.
- The PC announces itself with **mDNS** (service `_kinrelay._tcp`, named from a short hash of its public key, plus the protocol version) so the phone can find it. The PC's name is not announced; it is shown in the pairing QR code and sent to paired devices.
- Without a paired key the only thing a stranger on the local network can do is try to **pair**, and that needs the one-time token shown on the PC's screen (see above). Every other request needs a certificate whose key you paired.

## The browser extension's pipe

The browser extension talks to a small helper (`kin-native-host.exe`, started by the browser) that relays its requests to the running Kin app over a **Windows named pipe**. The pipe's access control list names the **current user only**, and remote clients are rejected. Any process running as the same Windows user could talk to it, which is the same trust boundary as everything else Kin has (such a process can already run `Kin.exe --send` or read your clipboard). The pipe's API is narrow: send, status and start; no unpairing, no settings, no history; and it is rate-limited.

## Developer flags in release builds

Release builds still contain `--dev-*` command-line flags that the end-to-end tests use. They are meant to be harmless:

- They **refuse to run without a separate `--data-dir`**, so they can never act on the profile of your installed Kin.
- Anything that listens on the network under a developer flag binds to loopback (`127.0.0.1`) only.
- They only matter to code that is already running as the same Windows user, which could do the same things without them.

If you find a way to use one of these flags against an installed Kin, that is a vulnerability: please report it.

## Out of scope

Attacks that need a malicious program already running as your Windows user or with root on your phone; someone with physical access to an unlocked device; denial of service by flooding your own local network; and the fact that the Windows installer is not yet code-signed (SmartScreen will warn you; check the checksum of a release yourself).
