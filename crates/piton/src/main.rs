use std::io::{self, Write as _};

fn main() -> io::Result<()> {
    writeln!(io::stdout().lock(), "Hello, world!")
}
