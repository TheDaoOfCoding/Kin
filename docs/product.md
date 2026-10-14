# Product

## The problem

Moving things between an Android phone and a Windows PC is still clumsy. People email files to themselves, message themselves on WhatsApp, or dig out a cable. Apple users don't think about it, because AirDrop, Universal Clipboard and Handoff just work.

Xiaomi, Samsung and others have their own versions, but they only work between their own devices. Someone with a Realme phone and a Dell laptop gets nothing.

## Who it's for

People who mix brands: any Android phone plus a Windows PC. Tablets, Macs and Linux come later.

## Pitch

"Your phone and your PC, finally acting like they belong to the same person. Any brand."

The big phone makers can't tell this story without weakening their own lock-in. That's our opening.

## Why we beat LocalSend

LocalSend is a good pipe: a file goes through and nothing stays behind. In the MVP we beat it on three things:

1. **Nothing to open.** You send from the share sheet, the text selection menu, the tray or a hotkey. The other side receives without anyone opening the app.
2. **Paired, not strangers.** You pair once with a QR code. After that your devices trust each other, so there are no accept prompts.
3. **Clipboard.** Copy on the PC, paste on the phone. Select text on the phone, send it, paste on the PC.

Later we add more: it works away from home (the bridge), keeps a history you can search, and includes small "it just knows" features (see [Phase 2](phases/phase-2-features.md)).

## What we learned from Quick Share

Google's Quick Share is used, but it never became a habit like AirDrop. It lost on reliability, fragmentation (several brands, several renames) and visibility, not on features. So reliability comes first, ahead of everything else.

## Principles

1. **Reliability over features.** Four features that always work beat ten that mostly work. A user should never see "device not found".
2. **Live in the OS, not in the app.** Share sheet, text selection menu, quick tile, notification, tray, hotkey. Daily users should rarely open the app.
3. **The phone's battery is sacred.** The PC is plugged in, so it does the heavy lifting.
4. **Private by default.** Everything is encrypted, there's no account, and in the MVP nothing leaves your network. No telemetry.
5. **Basic UI until it works.** Making it pretty is Phase 2.
6. **Measure, don't guess.** Every target in [targets.md](targets.md) has a way to measure it.

## Not goals (for now)

- **Sending to other people.** WhatsApp owns that. We're about your own devices. Receive-in-browser in Phase 2 is the one exception, as a growth loop.
- **Cloud storage or sync.** We move things between devices. We don't store them.
- **iPhone.** We have no device to build or test on.
