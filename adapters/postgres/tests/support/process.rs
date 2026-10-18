//! Native client termination, following the existing Fjall fixture protocol.

use std::io::BufRead;
#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

struct Process(Child);

impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

pub fn kill_after_marker(command: &mut Command, marker: &str) {
    let process = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    std::thread::scope(|scope| {
        let mut child = Process(process);
        let stdout = child.0.stdout.take().unwrap();
        let (sender, receiver) = mpsc::channel();
        let reader = scope.spawn(move || {
            for line in std::io::BufReader::new(stdout).lines() {
                if line.unwrap() == marker {
                    let _ = sender.send(());
                    break;
                }
            }
        });
        receiver
            .recv_timeout(Duration::from_secs(30))
            .expect("child must acknowledge the real commit");
        child.0.kill().unwrap();
        let status = child.0.wait().unwrap();
        #[cfg(unix)]
        assert_eq!(
            status.signal(),
            Some(9),
            "client must die from SIGKILL, not panic"
        );
        assert!(
            !status.success(),
            "client must retain handles until termination"
        );
        reader.join().unwrap();
    });
}
