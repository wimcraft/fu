//! Command-line and failure-path integration tests. These invoke the built
//! `fu` binary as a subprocess; none of them require a real terminal because
//! every scenario here fails before the picker/terminal stage is reached.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_fu"))
}

fn run(args: &[&str]) -> Output {
    Command::new(bin())
        .args(args)
        .env_remove("TMUX")
        .output()
        .expect("failed to run fu")
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

fn tiny_png(dir: &Path) -> PathBuf {
    let path = dir.join("tiny.png");
    let img = image::RgbImage::from_pixel(4, 4, image::Rgb([200, 100, 50]));
    img.save(&path).expect("write tiny png");
    path
}

fn scratch_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fu-cli-test-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

#[test]
fn help_exits_zero_and_documents_usage() {
    let output = run(&["--help"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("fu"));
}

#[test]
fn missing_image_argument_exits_two() {
    let output = run(&[]);
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn nonexistent_image_path_exits_nonzero_with_one_line_diagnostic() {
    let output = run(&["/nonexistent/path/does-not-exist.png"]);
    assert!(!output.status.success());
    let err = stderr(&output);
    assert!(err.starts_with("fu: "), "unexpected stderr: {err:?}");
    assert_eq!(err.lines().count(), 1);
}

#[test]
fn invalid_zoom_option_exits_two() {
    let dir = scratch_dir("zoom");
    let img = tiny_png(&dir);
    let output = run(&["-z", "0", img.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn non_finite_zoom_option_exits_two() {
    let dir = scratch_dir("zoom-nan");
    let img = tiny_png(&dir);
    let output = run(&["-z", "nan", img.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn invalid_overlap_option_exits_two() {
    let dir = scratch_dir("overlap");
    let img = tiny_png(&dir);
    let output = run(&["-o", "1.5", img.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn negative_overlap_option_exits_two() {
    let dir = scratch_dir("overlap-neg");
    let img = tiny_png(&dir);
    let output = run(&["-o", "-0.1", img.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn removed_bash_only_format_flag_exits_two() {
    let dir = scratch_dir("legacy-f");
    let img = tiny_png(&dir);
    let output = run(&["-f", "kitty", img.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn removed_bash_only_prefetch_flag_exits_two() {
    let dir = scratch_dir("legacy-p");
    let img = tiny_png(&dir);
    let output = run(&["-P", img.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn corrupt_image_exits_nonzero_with_one_line_diagnostic() {
    let dir = scratch_dir("corrupt");
    let path = dir.join("corrupt.png");
    let mut file = std::fs::File::create(&path).unwrap();
    file.write_all(b"\x89PNGnot actually a png, just garbage bytes")
        .unwrap();

    let output = run(&[path.to_str().unwrap()]);
    assert!(!output.status.success());
    let err = stderr(&output);
    assert!(err.starts_with("fu: "), "unexpected stderr: {err:?}");
    assert_eq!(err.lines().count(), 1);
}

#[test]
fn unsupported_image_format_exits_nonzero_with_one_line_diagnostic() {
    let dir = scratch_dir("unsupported");
    let path = dir.join("vector.svg");
    std::fs::write(&path, b"<svg xmlns='http://www.w3.org/2000/svg'></svg>").unwrap();

    let output = run(&[path.to_str().unwrap()]);
    assert!(!output.status.success());
    let err = stderr(&output);
    assert!(err.starts_with("fu: "), "unexpected stderr: {err:?}");
    assert_eq!(err.lines().count(), 1);
}

#[test]
fn tmux_passthrough_disabled_produces_the_exact_configuration_error() {
    let dir = scratch_dir("tmux-off");
    let img = tiny_png(&dir);

    let fake_tmux = dir.join("tmux");
    std::fs::write(&fake_tmux, "#!/bin/sh\necho off\nexit 0\n").unwrap();
    let mut perms = std::fs::metadata(&fake_tmux).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
    std::fs::set_permissions(&fake_tmux, perms).unwrap();

    let path_with_fake_tmux = format!("{}:{}", dir.display(), std::env::var("PATH").unwrap());

    let output = Command::new(bin())
        .arg(img.to_str().unwrap())
        .env("TMUX", "/tmp/tmux-1000/default,12345,0")
        .env("PATH", path_with_fake_tmux)
        .output()
        .expect("failed to run fu");

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        stderr(&output),
        "fu: tmux pixel graphics require: set -g allow-passthrough on\n"
    );
}
