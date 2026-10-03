use std::io::BufRead;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

struct KillOnDrop(Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Wait for an exact pipe marker, then terminate without dropping DB handles.
/// Cleanup also runs if the handshake times out or the assertions panic.
pub fn kill_after_marker(command: &mut Command, marker: &str) {
    let process = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    std::thread::scope(|scope| {
        let mut child = KillOnDrop(process);
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
            .expect("child must reach the handshake before termination");
        child.0.kill().unwrap();
        let status = child.0.wait().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            assert_eq!(
                status.signal(),
                Some(9),
                "child must exit from SIGKILL, not a test panic"
            );
        }
        assert!(
            !status.success(),
            "child must be terminated, not gracefully closed"
        );
        reader.join().unwrap();
    });
}
