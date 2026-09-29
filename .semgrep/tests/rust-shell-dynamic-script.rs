use std::process::Command;

fn positives(script: &str, other: String) {
    // ruleid: rust-shell-dynamic-script
    Command::new("/bin/zsh").args(["-lc", script]).output().unwrap();
    // ruleid: rust-shell-dynamic-script
    Command::new("/bin/sh").args(["-c", &other]).output().unwrap();
    // ruleid: rust-shell-dynamic-script
    std::process::Command::new("bash").args(["-c", &other]).output().unwrap();
}

fn negatives(path: &str) {
    // ok: rust-shell-dynamic-script
    Command::new("/bin/sh").args(["-c", "sleep 60 & wait"]).output().unwrap();
    // ok: rust-shell-dynamic-script
    Command::new("/usr/bin/git").args(["-C", path, "status"]).output().unwrap();
    // ok: rust-shell-dynamic-script
    Command::new("/bin/cp").args(["-c", path]).output().unwrap();
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    fn shell_inside_test_module(script: &str) {
        // ok: rust-shell-dynamic-script
        Command::new("/bin/sh").args(["-c", script]).output().unwrap();
    }
}
