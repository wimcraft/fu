//! Pseudo-terminal lifecycle tests: spawn the real `fu` binary attached to a
//! real pty, answer its Kitty-capability probe with a synthetic response (so
//! it selects the Kitty protocol without a real Ghostty behind it), then
//! drive it with keys/signals and confirm the pty's line discipline (raw
//! mode) is back to normal (canonical, echoing) after it exits.
//!
//! Terminal state is read via `TIOCGETA` on the *master* fd rather than by
//! spawning a second process on the pty: once a session leader with this pty
//! as its controlling terminal exits, BSD hangs up the *slave* side and any
//! further `login_tty()`-based spawn on the same slave fails with EBADF,
//! even though the fd is still nominally open in this process. The master
//! side is unaffected and keeps reporting live line-discipline state.
//!
//! This only covers the Ghostty-direct path (`$TMUX` unset); tmux passthrough
//! itself is exercised in `tests/cli.rs` and requires the manual Ghostty/tmux
//! acceptance run described in SPEC.md section 12.

use std::ffi::c_void;
use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::io::{AsRawFd, FromRawFd};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

extern "C" {
    fn openpty(
        amaster: *mut i32,
        aslave: *mut i32,
        name: *mut i8,
        termp: *mut c_void,
        winsize: *mut c_void,
    ) -> i32;
    fn login_tty(fd: i32) -> i32;
    fn close(fd: i32) -> i32;
    fn kill(pid: i32, sig: i32) -> i32;
    // `ioctl` must stay declared as C-variadic: on Apple's arm64 ABI, variadic
    // arguments are passed on the stack, unlike fixed arguments of the same
    // type, which go in registers. Declaring the trailing pointer as a fixed
    // parameter instead causes the callee to read garbage and fail with EFAULT.
    fn ioctl(fd: i32, request: u64, ...) -> i32;
}

const SIGINT: i32 = 2;
const SIGTERM: i32 = 15;

// From <sys/ttycom.h> / <sys/termios.h> on Darwin.
const TIOCSWINSZ: u64 = 0x8008_7467;
const TIOCGETA: u64 = 0x4048_7413;
const ICANON: u64 = 0x0000_0100;
const ECHO: u64 = 0x0000_0008;

#[repr(C)]
struct WinSize {
    ws_row: u16,
    ws_col: u16,
    ws_xpixel: u16,
    ws_ypixel: u16,
}

/// Mirrors Darwin's `struct termios` from `<sys/termios.h>`: four `u64`
/// flag words, a 20-byte control-char array, then two `u64` speeds.
#[repr(C)]
struct Termios {
    c_iflag: u64,
    c_oflag: u64,
    c_cflag: u64,
    c_lflag: u64,
    c_cc: [u8; 20],
    c_ispeed: u64,
    c_ospeed: u64,
}

impl Termios {
    fn zeroed() -> Self {
        Self {
            c_iflag: 0,
            c_oflag: 0,
            c_cflag: 0,
            c_lflag: 0,
            c_cc: [0; 20],
            c_ispeed: 0,
            c_ospeed: 0,
        }
    }

    fn is_cooked(&self) -> bool {
        self.c_lflag & ICANON != 0 && self.c_lflag & ECHO != 0
    }
}

/// `fork()` (used internally by `Command::spawn`) plus raw fd juggling isn't
/// safe to run concurrently across the OS threads `cargo test` uses for
/// separate `#[test]` functions in one binary, so serialize this file's
/// tests against each other.
static TEST_LOCK: Mutex<()> = Mutex::new(());

/// The exact `ratatui-image` capability-query response for a Kitty-capable,
/// non-tmux terminal with a 10x20 pixel cell: Kitty graphics OK, cell size
/// via `CSI 16 t`, then the terminating Device Status Report.
const FAKE_KITTY_CAPABILITY_RESPONSE: &[u8] = b"\x1b_Gi=31;OK\x1b\\\x1b[6;20;10t\x1b[0n";

struct Pty {
    master: File,
    slave_fd: i32,
    output: Arc<Mutex<Vec<u8>>>,
}

impl Pty {
    fn open() -> Self {
        let mut master_fd: i32 = -1;
        let mut slave_fd: i32 = -1;
        let rc = unsafe {
            openpty(
                &mut master_fd,
                &mut slave_fd,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        assert_eq!(rc, 0, "openpty failed: {}", std::io::Error::last_os_error());

        // A freshly opened pty reports a zero window size until someone sets
        // it (a real terminal emulator always does); without this, `fu`'s own
        // `terminal did not report pixel dimensions` guard correctly rejects it.
        let mut winsize = WinSize {
            ws_row: 24,
            ws_col: 80,
            ws_xpixel: 800,
            ws_ypixel: 480,
        };
        let rc = unsafe { ioctl(slave_fd, TIOCSWINSZ, &mut winsize as *mut WinSize) };
        assert_eq!(
            rc,
            0,
            "TIOCSWINSZ failed: {}",
            std::io::Error::last_os_error()
        );

        let master = unsafe { File::from_raw_fd(master_fd) };

        // Continuously drain the master side so the child never blocks on a
        // full pty output buffer (its Kitty escape sequences, frame redraws,
        // etc.), mirroring what a real terminal emulator does.
        let output = Arc::new(Mutex::new(Vec::new()));
        let captured_output = Arc::clone(&output);
        let mut reader = master.try_clone().expect("clone master for drain thread");
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => return,
                    Ok(count) => captured_output
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .extend_from_slice(&buf[..count]),
                }
            }
        });

        Self {
            master,
            slave_fd,
            output,
        }
    }

    /// Spawn `program` attached to this pty as its controlling terminal.
    /// Only valid to call once per `Pty`: once a session leader using this
    /// slave exits, BSD hangs up the slave side for everyone (see module docs).
    fn spawn(&self, program: &PathBuf, args: &[&str]) -> Child {
        let slave_fd = self.slave_fd;
        let mut cmd = Command::new(program);
        cmd.args(args);
        cmd.env_clear();
        cmd.env("PATH", std::env::var("PATH").unwrap_or_default());
        cmd.env("TERM", "xterm-kitty");
        cmd.stdin(Stdio::null());
        cmd.stdout(Stdio::null());
        cmd.stderr(Stdio::null());
        unsafe {
            cmd.pre_exec(move || {
                if login_tty(slave_fd) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        cmd.spawn().expect("spawn child attached to pty")
    }

    fn send_keys(&self, bytes: &[u8]) {
        (&self.master).write_all(bytes).expect("write keys to pty");
    }

    fn answer_capability_query(&self) {
        std::thread::sleep(Duration::from_millis(150));
        self.send_keys(FAKE_KITTY_CAPABILITY_RESPONSE);
    }

    fn take_output(&self) -> Vec<u8> {
        std::mem::take(&mut *self.output.lock().unwrap_or_else(|e| e.into_inner()))
    }

    fn wait_for_output(&self, needle: &[u8], timeout: Duration) -> Vec<u8> {
        let deadline = Instant::now() + timeout;
        let mut output = Vec::new();
        loop {
            output.extend(self.take_output());
            if output.windows(needle.len()).any(|window| window == needle) {
                return output;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for terminal output"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Read the pty's current line discipline via the master fd. Valid at
    /// any time, including after a previous session leader has exited.
    fn termios(&self) -> Termios {
        let mut termios = Termios::zeroed();
        let rc = unsafe {
            ioctl(
                self.master.as_raw_fd(),
                TIOCGETA,
                &mut termios as *mut Termios,
            )
        };
        assert_eq!(rc, 0, "TIOCGETA on master failed");
        termios
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        unsafe {
            close(self.slave_fd);
        }
    }
}

fn wait_for_exit(child: &mut Child, timeout: Duration) -> Option<std::process::ExitStatus> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            return Some(status);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn fu_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_fu"))
}

fn tiny_png() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fu-pty-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("tiny.png");
    let img = image::RgbImage::from_pixel(64, 256, image::Rgb([80, 120, 200]));
    img.save(&path).expect("write tiny png");
    path
}

#[test]
fn raw_mode_is_entered_then_restored_after_q() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let pty = Pty::open();
    let image = tiny_png();
    assert!(pty.termios().is_cooked(), "pty should start cooked");

    let mut child = pty.spawn(&fu_binary(), &[image.to_str().unwrap()]);
    pty.answer_capability_query();
    std::thread::sleep(Duration::from_millis(300));

    assert!(
        !pty.termios().is_cooked(),
        "fu should have entered raw mode by now"
    );

    pty.send_keys(b"q");
    let status = wait_for_exit(&mut child, Duration::from_secs(5));
    assert_eq!(status.map(|s| s.success()), Some(true));

    assert!(
        pty.termios().is_cooked(),
        "raw mode was not restored after q"
    );
}

#[test]
fn sigint_restores_the_terminal() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let pty = Pty::open();
    let image = tiny_png();
    let mut child = pty.spawn(&fu_binary(), &[image.to_str().unwrap()]);
    pty.answer_capability_query();
    std::thread::sleep(Duration::from_millis(300));
    assert!(!pty.termios().is_cooked(), "fu should be in raw mode");

    unsafe {
        kill(child.id() as i32, SIGINT);
    }

    let status = wait_for_exit(&mut child, Duration::from_secs(5));
    assert!(status.is_some(), "fu did not exit after SIGINT");
    assert!(
        pty.termios().is_cooked(),
        "raw mode was not restored after SIGINT"
    );
}

#[test]
fn sigterm_restores_the_terminal() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let pty = Pty::open();
    let image = tiny_png();
    let mut child = pty.spawn(&fu_binary(), &[image.to_str().unwrap()]);
    pty.answer_capability_query();
    std::thread::sleep(Duration::from_millis(300));
    assert!(!pty.termios().is_cooked(), "fu should be in raw mode");

    unsafe {
        kill(child.id() as i32, SIGTERM);
    }

    let status = wait_for_exit(&mut child, Duration::from_secs(5));
    assert!(status.is_some(), "fu did not exit after SIGTERM");
    assert!(
        pty.termios().is_cooked(),
        "raw mode was not restored after SIGTERM"
    );
}

#[test]
fn a_forty_key_burst_stays_responsive_and_exits_cleanly_on_q() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let pty = Pty::open();
    let image = tiny_png();
    let mut child = pty.spawn(&fu_binary(), &[image.to_str().unwrap()]);
    pty.answer_capability_query();
    std::thread::sleep(Duration::from_millis(300));

    let burst: Vec<u8> = b"hjkl".iter().cycle().take(40).copied().collect();
    pty.send_keys(&burst);
    pty.send_keys(b"q");

    let status = wait_for_exit(&mut child, Duration::from_secs(5));
    assert_eq!(
        status.map(|s| s.success()),
        Some(true),
        "fu did not exit cleanly after a 40-key burst"
    );
    assert!(
        pty.termios().is_cooked(),
        "raw mode was not restored after the burst"
    );
}

#[test]
fn scrolling_replaces_the_placement_without_retransmitting_pixels() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let pty = Pty::open();
    let image = tiny_png();
    let mut child = pty.spawn(&fu_binary(), &[image.to_str().unwrap()]);
    pty.answer_capability_query();

    pty.wait_for_output(b"_Ga=p", Duration::from_secs(2));
    std::thread::sleep(Duration::from_millis(50));
    pty.take_output();

    pty.send_keys(b"j");
    let output = pty.wait_for_output(b"_Ga=p", Duration::from_secs(2));
    assert!(
        !output.windows(3).any(|window| window == b"a=t"),
        "scrolling retransmitted source pixels"
    );

    pty.send_keys(b"q");
    let status = wait_for_exit(&mut child, Duration::from_secs(5));
    assert_eq!(status.map(|s| s.success()), Some(true));
}

#[test]
fn fit_key_places_the_entire_portrait_image_with_height_as_the_limit() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let pty = Pty::open();
    let image = tiny_png();
    let mut child = pty.spawn(&fu_binary(), &[image.to_str().unwrap()]);
    pty.answer_capability_query();

    pty.wait_for_output(b"_Ga=p", Duration::from_secs(2));
    std::thread::sleep(Duration::from_millis(50));
    pty.take_output();

    pty.send_keys(b"f");
    let output = pty.wait_for_output(b"_Ga=p", Duration::from_secs(2));
    assert!(
        output
            .windows(b"x=0,y=0,w=64,h=256,r=22".len())
            .any(|window| window == b"x=0,y=0,w=64,h=256,r=22"),
        "fit did not place the full image against the pane height"
    );

    pty.send_keys(b"q");
    let status = wait_for_exit(&mut child, Duration::from_secs(5));
    assert_eq!(status.map(|s| s.success()), Some(true));
}
