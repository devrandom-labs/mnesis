//! Native server lifecycle shared by restore and secure-connection tests.

use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use tempfile::TempDir;

pub struct Cluster {
    directory: TempDir,
}

impl Cluster {
    pub(super) fn new() -> Self {
        let directory = tempfile::Builder::new()
            .prefix("mnesis-restore-")
            .tempdir_in("/tmp")
            .unwrap();
        Self { directory }
    }

    pub(super) fn initialize() -> Self {
        let cluster = Self::new();
        let initialized = Command::new("initdb")
            .args(["--no-sync", "--auth=trust", "-U", "mnesis", "-D"])
            .arg(cluster.data())
            .output()
            .unwrap();
        assert!(initialized.status.success(), "{initialized:?}");
        cluster
    }

    pub(super) fn data(&self) -> PathBuf {
        self.directory.path().join("data")
    }

    pub(super) fn socket(&self) -> &Path {
        self.directory.path()
    }

    pub(super) fn launch(&self, options: &str) {
        let started = Command::new("pg_ctl")
            .arg("-D")
            .arg(self.data())
            .arg("-l")
            .arg(self.directory.path().join("postgres.log"))
            .args(["-o", options, "-w", "start"])
            .output()
            .unwrap();
        assert!(started.status.success(), "{started:?}");
    }

    pub(super) fn stop(&self) -> io::Result<Output> {
        Command::new("pg_ctl")
            .arg("-D")
            .arg(self.data())
            .args(["-m", "fast", "-w", "stop"])
            .output()
    }
}

impl Drop for Cluster {
    fn drop(&mut self) {
        let _stopped = self.stop();
    }
}
