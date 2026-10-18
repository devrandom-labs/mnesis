use std::io::{self, Write};

fn main() -> io::Result<()> {
    writeln!(io::stdout().lock(), "cargo:rerun-if-changed=migrations")
}
