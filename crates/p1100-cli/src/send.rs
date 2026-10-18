//! Handing an encoded job file to Apple's own CUPS backend.

use std::env;
use std::process::Command;

/// Apple's own CUPS backend: the transport a queue uses, world-executable, and the only thing
/// that has to run to put a job on the wire. `p1100 send` wraps it so the bring-up notes are one
/// command rather than a DEVICE_URI dance.
const USB_BACKEND: &str = "/usr/libexec/cups/backend/usb";
/// Send a job file to the printer through Apple's own backend, the transport a queue would use.
/// No queue and no administrator: the backend takes its device URI from the environment and
/// reads the job from the file argument, as backend(7) describes.
pub(crate) fn send(args: &[String]) -> Result<(), String> {
    let mut job: Option<String> = None;
    let mut device: Option<String> = None;
    let mut queue = "HP_LaserJet_Professional_P1102".to_string();
    let mut timeout = 60u64;
    let mut dry = false;
    let mut i = 0;
    while i < args.len() {
        let need = |i: usize| -> Result<&String, String> {
            args.get(i + 1)
                .ok_or_else(|| format!("{} needs a value", args[i]))
        };
        match args[i].as_str() {
            "--device" => {
                device = Some(need(i)?.clone());
                i += 2;
            }
            "--queue" => {
                queue = need(i)?.clone();
                i += 2;
            }
            "--timeout" => {
                timeout = need(i)?.parse().map_err(|_| "bad --timeout")?;
                i += 2;
            }
            "--dry-run" => {
                dry = true;
                i += 1;
            }
            other if other.starts_with('-') => return Err(format!("unknown option {other:?}")),
            name => {
                if job.is_some() {
                    return Err("send takes one job file".into());
                }
                job = Some(name.to_string());
                i += 1;
            }
        }
    }
    let job = job.ok_or("send needs a job file: p1100 send <job.zjs> [--device URI]")?;
    if !dry && !std::path::Path::new(&job).exists() {
        return Err(format!("no such job file: {job}"));
    }
    let device = match device {
        Some(d) => d,
        None => {
            // The queue already knows the device URI, and reading it is what the bring-up notes
            // do by hand: lpstat -v prints "device for <queue>: <uri>".
            let out = Command::new("lpstat")
                .args(["-v", &queue])
                .output()
                .map_err(|e| format!("cannot run lpstat: {e}"))?;
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .find_map(|l| l.split_once(": "))
                .map(|(_, uri)| uri.trim().to_string())
                .ok_or_else(|| format!("queue {queue} has no device URI; pass --device URI"))?
        }
    };
    let user = env::var("USER").unwrap_or_else(|_| "user".to_string());
    let argv = ["1", user.as_str(), "p1100 send", "1", "", job.as_str()];
    if dry {
        println!("DEVICE_URI={device} {USB_BACKEND} {}", argv.join(" "));
        return Ok(());
    }
    eprintln!("sending {job} to {device} ({timeout}s limit; Ctrl-C to stop)");
    let mut child = Command::new(USB_BACKEND)
        .args(argv)
        .env("DEVICE_URI", &device)
        .spawn()
        .map_err(|e| format!("cannot run {USB_BACKEND}: {e}"))?;
    let started = std::time::Instant::now();
    loop {
        match child
            .try_wait()
            .map_err(|e| format!("waiting for the backend: {e}"))?
        {
            Some(status) if status.success() => {
                eprintln!("the backend accepted the job");
                return Ok(());
            }
            Some(status) => return Err(format!("the backend exited with {status}")),
            None => {
                if started.elapsed() >= std::time::Duration::from_secs(timeout) {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!(
                        "still going after {timeout}s, so the printer is not taking the job - \
                         which is what the backend says with no printer attached, as the \
                         bring-up section of docs/testing.md records"
                    ));
                }
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
        }
    }
}
#[cfg(test)]
mod send_tests {
    use super::*;

    #[test]
    fn send_takes_one_job_and_refuses_the_rest() {
        let args = |v: &[&str]| -> Vec<String> { v.iter().map(|s| s.to_string()).collect() };

        // A dry run needs no printer and no device: the URI comes from --device.
        send(&args(&["job.zjs", "--dry-run", "--device", "usb://x"])).expect("a dry run");

        let err = send(&args(&[])).unwrap_err();
        assert!(err.contains("needs a job file"), "{err}");

        let err = send(&args(&["a.zjs", "b.zjs", "--dry-run", "--device", "u"])).unwrap_err();
        assert!(err.contains("one job file"), "{err}");

        let err = send(&args(&["job.zjs", "--wat"])).unwrap_err();
        assert!(err.contains("--wat"), "{err}");

        // A file that is not there is refused before anything is spawned.
        let err = send(&args(&["/nonexistent/job.zjs", "--device", "usb://x"])).unwrap_err();
        assert!(err.contains("no such job file"), "{err}");
    }
}
