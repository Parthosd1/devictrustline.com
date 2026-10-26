//! DeviceTrustline device companion.
//!
//! Finds the hardware on this computer (the computer itself, monitors, docks, printers, disks and
//! USB devices) and sends the list to one DeviceTrustline workspace, where people review it.
//! It only reports what the system says: it never makes up serial numbers, purchase dates or
//! warranties, and nothing becomes an item until someone adds it in the web app.

mod client;
mod collect;
mod config;
mod device;

use std::time::Duration;

use client::Failure;

const USAGE: &str = "DeviceTrustline device companion

Usage:
  devicetrustline-agent pair <server> <code> [--name <computer name>]
                                 Pair this computer with a workspace (code from Devices > Pair a computer)
  devicetrustline-agent scan     Show what would be sent, without sending anything
  devicetrustline-agent report   Scan once and send the result
  devicetrustline-agent run [--every <minutes>]
                                 Keep running: scan and send every 60 minutes (default)
  devicetrustline-agent status   Show pairing and connection status
  devicetrustline-agent pause    Stop sending until 'resume' (an admin can also pause it from the web app)
  devicetrustline-agent resume
  devicetrustline-agent disconnect
                                 Revoke this computer's credential and forget the pairing
  devicetrustline-agent install  Windows: run hourly in the background (scheduled task, as SYSTEM)
  devicetrustline-agent uninstall
                                 Windows: remove the scheduled task, then disconnect
";

fn fail(message: impl std::fmt::Display) -> ! {
    eprintln!("{message}");
    std::process::exit(1);
}

fn hostname() -> String {
    std::env::var("COMPUTERNAME")
        .ok()
        .or_else(|| std::fs::read_to_string("/proc/sys/kernel/hostname").ok())
        .or_else(|| std::env::var("HOSTNAME").ok())
        .map(|h| h.trim().to_string())
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| "This computer".into())
}

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn report_once(c: &config::Config) -> Result<String, Failure> {
    let devices = collect::collect().map_err(Failure::Retry)?;
    let res = client::report(c, &devices)?;
    Ok(if res["paused"] == true {
        "Paused from the web app; nothing was sent.".to_string()
    } else {
        format!(
            "Sent {} devices to {} ({} moved here, {} no longer seen).",
            res["accepted"], c.workspace, res["moved"], res["missing"]
        )
    })
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("pair") => {
            let (Some(server), Some(code)) = (args.get(1), args.get(2)) else { fail(USAGE) };
            let server = config::check_server(server).unwrap_or_else(|e| fail(e));
            let name = flag(&args, "--name").unwrap_or_else(hostname);
            let res = client::pair(&server, code, &name, collect::platform()).unwrap_or_else(|e| fail(e));
            let c = config::Config {
                server,
                agent_id: res["agentId"].as_str().unwrap_or("").into(),
                token: res["token"].as_str().unwrap_or("").into(),
                workspace: res["workspace"].as_str().unwrap_or("").into(),
                paused: false,
            };
            config::save(&c).unwrap_or_else(|e| fail(e));
            println!("Paired {name} with {}. Run 'report' to send its devices now.", c.workspace);
        }
        Some("scan") => {
            let devices = collect::collect().unwrap_or_else(|e| fail(e));
            println!("{}", serde_json::to_string_pretty(&devices).unwrap());
            eprintln!("{} devices found. Nothing was sent.", devices.len());
        }
        Some("report") => {
            let c = config::load().unwrap_or_else(|e| fail(e));
            if c.paused {
                fail("Paused on this computer. Run 'resume' first.");
            }
            println!("{}", report_once(&c).unwrap_or_else(|e| fail(e)));
        }
        Some("run") => {
            let every = flag(&args, "--every").and_then(|m| m.parse::<u64>().ok()).unwrap_or(60).max(5);
            let mut backoff = 30;
            loop {
                let c = config::load().unwrap_or_else(|e| fail(e));
                let wait = if c.paused {
                    every * 60
                } else {
                    match report_once(&c) {
                        Ok(m) => {
                            println!("{m}");
                            backoff = 30;
                            every * 60
                        }
                        Err(Failure::Unpaired(m)) => fail(format!("{m} Stopping. Pair again to continue.")),
                        Err(e) => {
                            // Reconnects on its own, waiting longer each time (up to 30 minutes).
                            eprintln!("{e}. Trying again in {backoff} seconds.");
                            let w = backoff;
                            backoff = (backoff * 2).min(1800);
                            w
                        }
                    }
                };
                std::thread::sleep(Duration::from_secs(wait));
            }
        }
        Some("status") => {
            let c = config::load().unwrap_or_else(|e| fail(e));
            println!("Server: {}\nWorkspace: {}\nPaused here: {}", c.server, c.workspace, c.paused);
            match client::status(&c) {
                Ok(s) => {
                    println!("Connection: ok{}", if s["paused"] == true { " (paused from the web app)" } else { "" })
                }
                Err(e) => println!("Connection: {e}"),
            }
        }
        Some(cmd @ ("pause" | "resume")) => {
            let mut c = config::load().unwrap_or_else(|e| fail(e));
            c.paused = cmd == "pause";
            config::save(&c).unwrap_or_else(|e| fail(e));
            println!("{}", if c.paused { "Paused. Nothing will be sent until you run 'resume'." } else { "Resumed." });
        }
        Some("disconnect") => disconnect(),
        Some("install") => install(),
        Some("uninstall") => {
            uninstall_task();
            disconnect();
        }
        _ => {
            print!("{USAGE}");
            if !args.is_empty() && args[0] != "help" && args[0] != "--help" {
                std::process::exit(2);
            }
        }
    }
}

fn disconnect() {
    let Ok(c) = config::load() else {
        println!("This computer is not paired.");
        return;
    };
    match client::disconnect(&c) {
        Ok(_) | Err(Failure::Unpaired(_)) => println!("Disconnected from {}.", c.workspace),
        Err(e) => eprintln!("Could not tell the server ({e}). Remove this computer under Devices in the web app."),
    }
    config::forget();
}

const TASK: &str = "DeviceTrustline Agent";

fn install() {
    if !cfg!(windows) {
        fail("Background install is automated on Windows. On Linux or macOS, run 'devicetrustline-agent report' from cron, a systemd timer or launchd (see the README).");
    }
    config::load().unwrap_or_else(|e| fail(e));
    let exe = std::env::current_exe().unwrap_or_else(|e| fail(e));
    let action = format!("\"{}\" report", exe.display());
    let status = std::process::Command::new("schtasks")
        .args(["/Create", "/TN", TASK, "/TR", &action, "/SC", "HOURLY", "/RU", "SYSTEM", "/RL", "HIGHEST", "/F"])
        .status()
        .unwrap_or_else(|e| fail(format!("could not run schtasks: {e}")));
    if !status.success() {
        fail("Creating the scheduled task failed. Run this from an administrator command prompt.");
    }
    println!("Installed: this computer reports every hour. Remove with 'uninstall'.");
}

fn uninstall_task() {
    if cfg!(windows) {
        let _ = std::process::Command::new("schtasks").args(["/Delete", "/TN", TASK, "/F"]).status();
    }
}
