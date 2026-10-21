# Kin Relay documentation

**Kin Relay** links an Android phone and a Windows PC over your local network. Inside the apps it is just **Kin**; technical identifiers use `kinrelay`. For what it does and how to install it, start with the [root README](../README.md).

| Document | What it covers |
|---|---|
| [product.md](product.md) | What Kin is for, who it is for, and its principles |
| [architecture.md](architecture.md) | How the phone and the PC fit together: identity, discovery, connections, the queue |
| [protocol.md](protocol.md) | The exact messages between phone and PC (v1), with a changelog of every rule change |
| [android.md](android.md) | Design of the Android app |
| [windows.md](windows.md) | Design of the Windows app, the installer and the tray |
| [extension.md](extension.md) | The browser extension and the native messaging host |
| [targets.md](targets.md) | What Kin optimises for and how it is measured |

Related files in the repository:

- [protocol/](../protocol/README.md): example messages and test vectors that both apps run, and the reference checker.
- [CONTRIBUTING.md](../CONTRIBUTING.md): how to build and test every part.
- [SECURITY.md](../SECURITY.md): the security model and how to report a vulnerability.
- [CHANGELOG.md](../CHANGELOG.md): what changed in each release.

When behaviour on the wire changes, update [protocol.md](protocol.md) and the vectors in `protocol/` in the same change.
