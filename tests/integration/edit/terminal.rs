// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-License-Identifier: MulanPSL-2.0

//! Selected values are editable in the terminal, without an external editor.

use super::{SPEC, command, fixture};
use std::fs;

#[test]
fn selected_inline_edit_rejects_nonterminal_without_creating_stage() {
    let directory = fixture(SPEC);
    for format in [false, true] {
        let mut request = command(directory.path());
        request.args(["--spec=ed.spec", "--field=package.version"]);
        if format {
            request.arg("--format=toml");
        }
        let output = request.output().unwrap();
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        let message = if format {
            let report = super::support::machine_report(&output);
            assert!(output.stderr.is_empty());
            report["error"]["message"].as_str().unwrap().to_owned()
        } else {
            assert!(output.stdout.is_empty());
            String::from_utf8(output.stderr).unwrap()
        };
        assert!(message.contains("--set FIELD=VALUE"), "{message}");
        assert!(!directory.path().join(".ruyipack-stage").exists());
        assert_eq!(
            fs::read_to_string(directory.path().join("ed.spec")).unwrap(),
            SPEC
        );
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn menu_and_selected_field_edit_existing_values_in_a_real_terminal() {
    // Wait for the terminal's raw input state, not a fixed sleep: early backspaces
    // would otherwise be consumed by the terminal before the prompt reads them.
    const TERMINAL: &str = r#"
import errno, fcntl, os, pty, select, signal, struct, sys, termios, time
binary, mode = sys.argv[1:]
args = [binary, 'edit', '--spec=ed.spec', mode, '--diff']
pid, fd = pty.fork()
if pid == 0:
    os.environ.update(TERM='xterm', NO_COLOR='1', EDITOR="sh -c 'touch editor-was-run; exit 99'", VISUAL="sh -c 'touch editor-was-run; exit 99'")
    os.execv(binary, args)
fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack('HHHH', 24, 120, 0, 0))
output = bytearray()
menu_selected, value_entered = mode != '--menu', False
deadline = time.monotonic() + 15
try:
    while time.monotonic() < deadline:
        if select.select([fd], [], [], .01)[0]:
            try:
                chunk = os.read(fd, 65536)
            except OSError as error:
                if error.errno == errno.EIO: break
                raise
            if not chunk: break
            output.extend(chunk)
        if not menu_selected and b'What do you want to edit?' in output:
            os.write(fd, b'\r')
            menu_selected = True
        if not value_entered and b'package.version: 1.22.5' in output and not termios.tcgetattr(fd)[3] & termios.ICANON:
            os.write(fd, b'\x7f' * 6 + b'1.22.6\r')
            value_entered = True
    else:
        os.killpg(pid, signal.SIGKILL)
        raise RuntimeError('terminal editing timed out: ' + repr(output))
    _, status = os.waitpid(pid, 0)
finally:
    os.close(fd)
sys.stdout.buffer.write(output)
sys.exit(os.waitstatus_to_exitcode(status))
"#;
    for mode in ["--menu", "--field=package.version"] {
        let directory = fixture(SPEC);
        let output = std::process::Command::new("python3")
            .current_dir(directory.path())
            .args(["-c", TERMINAL, env!("CARGO_BIN_EXE_ruyipack"), mode])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let transcript = String::from_utf8_lossy(&output.stdout);
        assert!(
            transcript.contains("package.version: 1.22.5"),
            "{transcript}"
        );
        assert!(
            transcript.contains("package.version: 1.22.6"),
            "{transcript}"
        );
        let stage = directory.path().join(".ruyipack-stage/ed");
        let values: toml::Value =
            toml::from_str(&fs::read_to_string(stage.join("ed.toml")).unwrap()).unwrap();
        assert_eq!(values["package"]["version"].as_str(), Some("1.22.6"));
        assert!(!directory.path().join("editor-was-run").exists());
        assert_eq!(
            fs::read_to_string(directory.path().join("ed.spec")).unwrap(),
            SPEC
        );
        assert_eq!(
            fs::read_to_string(stage.join("ed.candidate.spec")).unwrap(),
            super::version_source("1.22.6")
        );
    }
}
