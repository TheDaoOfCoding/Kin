## What and why

<!-- What does this change, and why? Link the issue it fixes, if there is one. -->

## How I tested it

<!-- Which tests you ran (see CONTRIBUTING.md), and on which phone or emulator and which Windows version. -->

## Checklist

- [ ] The tests of the parts I changed pass (`cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` for Rust; `gradlew testDebugUnitTest` for Android; `bun run typecheck` and `bun test tests/unit` for the extension; `bun protocol/check-vectors.ts`).
- [ ] If I changed behaviour on the wire, I updated `docs/protocol.md` and the vectors in `protocol/`.
- [ ] I added or updated tests for the change.
- [ ] No secrets, keys, pairing codes or private logs are included.
- [ ] **I have read the [Contributor License Agreement](../CLA.md) and, by submitting this pull request, I agree to its terms** (I keep my copyright, and the project may license my contribution under the AGPL-3.0 and under commercial terms).
