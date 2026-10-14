# tools

Scripts for soak tests and benchmarks. Measured results go in `docs/results/`, not here.

## net-check.ts

Phase 0, step 5 in [phase-0-setup.md](../docs/phases/phase-0-setup.md): checks that a phone on Wi-Fi can reach this PC.

```
bun tools/net-check.ts
bun tools/net-check.ts --minutes 30
```

- It serves "Kin Relay network check OK" and the PC's name on port 47821, on all interfaces.
- On start it prints a ready-to-type URL for each LAN address, and marks the one on the default route when there is more than one. It skips loopback, link-local and virtual adapters (Hyper-V/WSL, VirtualBox, VMware, Docker, VPNs), and lists what it skipped. It never skips the address on the default route, even if the adapter's name looks virtual, because a Hyper-V external switch (`vEthernet (...)`) can be the real LAN.
- It logs every request with the time and the remote address. Browsers also ask for `/favicon.ico`, so expect two lines per page load.
- It stops on Ctrl+C, or after 10 minutes. Change that with `--minutes N`.

How to use it:

1. Run it. The first time, Windows shows a firewall prompt. Allow it, and note whether it says Private or Public (this feeds D12).
2. On the phone, turn on Wi-Fi and turn off mobile data, on the same network as the PC. Open the printed URL in the browser.
3. You should see "Kin Relay network check OK", and a log line with the phone's address.

If the page doesn't load:

- The phone or the PC is on a guest network.
- AP or client isolation is on in the router, so Wi-Fi devices can't talk to each other.
- The firewall prompt was dismissed or closed. Windows then blocks the program silently, and running it again won't ask again. Allow `bun.exe` for the network (Private and Public) in Windows Security → Firewall & network protection → Allow an app through firewall.
- You used the wrong address. Try the other URLs that were printed.

## Removing the firewall rule afterwards

When you allow the Windows firewall prompt, Windows creates inbound rules for `bun.exe` (usually named "bun" or "bun.exe", one for TCP and one for UDP). They stay after the test. Removing them needs admin.

- **Settings route:** Windows Defender Firewall → Advanced settings → Inbound Rules → select the "bun" entries → Delete.
- **PowerShell route:** open PowerShell as administrator and run:

  ```
  Get-NetFirewallApplicationFilter | Where-Object { $_.Program -like '*\bun.exe' } | Get-NetFirewallRule | Remove-NetFirewallRule
  ```

  To see what it will remove first, run the same line with `| Select-Object DisplayName, Direction, Profile` instead of `| Remove-NetFirewallRule`.

This removes every inbound rule for `bun.exe`, not only this test's. Windows asks again the next time a Bun program listens on the network.
