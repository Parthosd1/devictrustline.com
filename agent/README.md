# DeviceTrustline device companion

A small Rust program that finds the hardware on a computer and sends the list to one
DeviceTrustline workspace, where people review it under **Devices**. Nothing becomes an item
until someone adds it or links it to an existing item.

It reports only what the operating system says. It never makes up serial numbers, purchase
dates or warranties. A device without a serial is recognized by the computer that reported it
plus the system's own id for it, so two identical mice on two desks stay two mice. A device with
a real serial is recognized wherever it turns up, so a monitor moved to another desk shows as
moved.

## What it reads

| Platform | Source | Finds |
| --- | --- | --- |
| Windows 10/11 | PowerShell `Get-CimInstance` (CIM/WMI, built in) | the computer and its serial, monitors (with EDID serials), disks, printers, USB cameras, mice, keyboards, phones, USB network adapters and docks |
| Linux | sysfs (`/sys/class/dmi`, `/sys/bus/usb`) | the computer (serial needs root), USB devices |
| macOS | `system_profiler -json` | the Mac and its serial, external displays, USB devices |

Windows is the first supported platform. The Linux and macOS readers work but are simpler.

## Use

```sh
# In the web app: Devices > Pair a computer gives a one-time code (valid 15 minutes).
devicetrustline-agent pair https://devicetrustline.com ABCDE-FGHJK
devicetrustline-agent scan        # show what would be sent; sends nothing
devicetrustline-agent report      # scan once and send
devicetrustline-agent install     # Windows, as administrator: report every hour (scheduled task)
devicetrustline-agent status
devicetrustline-agent pause       # or pause it from the web app
devicetrustline-agent resume
devicetrustline-agent uninstall   # remove the scheduled task and disconnect
devicetrustline-agent disconnect  # revoke this computer's credential and forget the pairing
```

On Linux or macOS, schedule `devicetrustline-agent report` with cron, a systemd timer or launchd
instead of `install`. `run` keeps a process running and retries with growing waits (30 seconds
up to 30 minutes) when the server cannot be reached.

The pairing is stored in `%ProgramData%\DeviceTrustline\agent.json` on Windows and
`~/.config/devicetrustline/agent.json` elsewhere (override with `DT_AGENT_CONFIG`). It holds a
credential that only reaches this computer's own device endpoints in one workspace; revoking the
computer in the web app (or `disconnect`) stops it at once. The server address must use HTTPS.
`HTTPS_PROXY` and `NO_PROXY` are honored.

## Build

```sh
cargo build --release                                   # this platform
rustup target add x86_64-pc-windows-gnu                 # Windows, from Linux (needs mingw-w64)
cargo build --release --target x86_64-pc-windows-gnu    # target/x86_64-pc-windows-gnu/release/devicetrustline-agent.exe
```

CI builds the Windows executable on every push and keeps it as the `devicetrustline-agent-windows-x64` artifact.

## How it was tested

| Level | What | Status |
| --- | --- | --- |
| Simulated | Unit tests parse recorded Windows CIM output, macOS `system_profiler` output and a fake sysfs tree (`cargo test`) | passing |
| Real program, real server | The HTTP test suite runs the Linux build against the API: pair, scan, report, local pause, disconnect | passing |
| Compiled for Windows | `x86_64-pc-windows-gnu` release build and clippy for that target | passing |
| On a physical Windows PC | Running the `.exe`, the PowerShell query, `install` / `uninstall` | **not yet verified** — needs a Windows 10/11 machine |
| macOS | Running on a Mac | **not yet verified** |
