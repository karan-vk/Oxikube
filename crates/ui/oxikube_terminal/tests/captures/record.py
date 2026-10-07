#!/usr/bin/env python3
"""Records the bytes a full-screen program writes to its terminal (E09-S13).

    python3 record.py vim|less|tmux

Writes `<name>.vt` next to this script: the raw output of the program on an 80x24 xterm-256color
pty, with a fixed scripted session (no host name, user name, clock or path in it). The tests feed
the file through the grid and a screenshot; the recording is committed so nobody needs the
program installed. Re-record only when a capture should show something else.
"""
import fcntl, os, pty, select, struct, sys, tempfile, termios, time

COLS, ROWS = 80, 24
HERE = os.path.dirname(os.path.abspath(__file__))

SAMPLE_RS = """\
//! A small sample for the capture.
use std::collections::HashMap;

/// Counts words.
fn count(text: &str) -> HashMap<&str, usize> {
    let mut seen = HashMap::new();
    for word in text.split_whitespace() {
        *seen.entry(word).or_insert(0) += 1;
    }
    seen
}

fn main() {
    let words = count("to be or not to be");
    println!("{:?}", words.get("be")); // Some(2)
}
"""

LESS_TXT = "".join(
    "\x1b[1;32mINFO \x1b[0m line %02d: pod web-%d is \x1b[33mPending\x1b[0m -> \x1b[32mRunning\x1b[0m\n" % (n, n)
    for n in range(1, 41)
)

TMUX_CONF = """\
set -g status-style "bg=blue,fg=white"
set -g status-left "[demo] "
set -g status-right "load 0.42"
set -g status-interval 0
set -g pane-border-style "fg=colour240"
set -g pane-active-border-style "fg=green"
"""

PANE_TOP = (
    "printf '\\033[1m CPU[\\033[32m||||||||\\033[33m||||\\033[31m||\\033[0m      58%%]\\033[0m\\n';"
    "printf '\\033[1m Mem[\\033[34m||||||||||\\033[0m      3.1G/8.0G]\\033[0m\\n';"
    "printf '\\033[30;46m  PID USER      CPU%% COMMAND   \\033[0m\\n';"
    "printf '    1 root       0.0 init\\n  412 app       41.2 \\033[1mkubelet\\033[0m\\n';"
    "sleep 30"
)

PANE_LOG = (
    "printf '\\033[1;32mINFO \\033[0m web-1 \\033[32mRunning\\033[0m\\n';"
    "printf '\\033[1;33mWARN \\033[0m web-2 \\033[33mPending\\033[0m\\n';"
    "printf '\\033[1;31mERROR\\033[0m web-3 \\033[31mCrashLoop\\033[0m\\n';"
    "printf '\\033[38;5;208mwide: \\xe4\\xbd\\xa0\\xe5\\xa5\\xbd\\033[0m\\n';"
    "sleep 30"
)


def run(argv, script, env_extra=None, cwd=None):
    env = {"TERM": "xterm-256color", "LANG": "C.UTF-8", "LC_ALL": "C.UTF-8", "PATH": os.environ["PATH"], "HOME": cwd or "/", "SHELL": "/bin/sh"}
    env.update(env_extra or {})
    pid, fd = pty.fork()
    if pid == 0:
        if cwd:
            os.chdir(cwd)
        os.execvpe(argv[0], argv, env)
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLS, 0, 0))
    out = bytearray()

    def pump(seconds):
        end = time.time() + seconds
        while time.time() < end:
            ready, _, _ = select.select([fd], [], [], 0.05)
            if ready:
                try:
                    data = os.read(fd, 65536)
                except OSError:
                    return False
                if not data:
                    return False
                out.extend(data)
        return True

    pump(1.0)
    for keys, wait in script:
        os.write(fd, keys.encode())
        if not pump(wait):
            break
    pump(0.5)
    try:
        os.kill(pid, 9)
    except OSError:
        pass
    os.waitpid(pid, 0)
    return bytes(out)


def main(name):
    work = tempfile.mkdtemp()
    if name == "vim":
        open(os.path.join(work, "sample.rs"), "w").write(SAMPLE_RS)
        data = run(
            ["vim", "-u", "NONE", "-U", "NONE", "-N", "-n", "-i", "NONE", "sample.rs"],
            [(":syntax on\r", 0.6), (":set number\r", 0.4), ("/entry\r", 0.4), ("jjdd", 0.4)],
            cwd=work,
        )
    elif name == "less":
        open(os.path.join(work, "log.txt"), "w").write(LESS_TXT)
        data = run(["less", "-R", "log.txt"], [(" ", 0.4)], cwd=work)
    elif name == "tmux":
        open(os.path.join(work, "tmux.conf"), "w").write(TMUX_CONF)
        sock = os.path.join(work, "sock")
        data = run(
            ["tmux", "-S", sock, "-f", "tmux.conf", "new-session", "-s", "demo", "sh", "-c", PANE_TOP,
             ";", "split-window", "-h", "sh", "-c", PANE_LOG],
            [("", 0.8)],
            cwd=work,
        )
        os.system("tmux -S %s kill-server >/dev/null 2>&1" % sock)
    else:
        sys.exit("unknown program " + name)
    path = os.path.join(HERE, name + ".vt")
    open(path, "wb").write(data)
    print(path, len(data), "bytes")


main(sys.argv[1])
