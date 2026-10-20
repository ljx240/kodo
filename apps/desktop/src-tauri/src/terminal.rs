//! Bottom terminal: one PTY-backed interactive shell session per panel.
//!
//! Output is streamed to the webview as `terminal:data` events; exit is
//! reported as `terminal:exit`. Sessions are keyed by a UUID and removed
//! either by `terminal_close` or by the waiter thread when the shell exits.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;

use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use tauri::{AppHandle, Emitter, State};

pub struct Session {
    writer: Box<dyn Write + Send>,
    master: Box<dyn MasterPty + Send>,
    killer: Box<dyn ChildKiller + Send + Sync>,
    pid: Option<u32>,
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.killer.kill();
        kill_group(self.pid);
    }
}

#[derive(Default, Clone)]
pub struct Terminals(Arc<Mutex<HashMap<String, Session>>>);

impl Terminals {
    fn lock(&self) -> MutexGuard<'_, HashMap<String, Session>> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct TerminalData {
    id: String,
    data: String,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct TerminalExit {
    id: String,
    code: Option<u32>,
}

fn emit_data(app: &AppHandle, id: &str, data: String) {
    if data.is_empty() {
        return;
    }
    let _ = app.emit(
        "terminal:data",
        TerminalData {
            id: id.to_owned(),
            data,
        },
    );
}

/// Emit everything that is valid UTF-8, keeping an incomplete trailing
/// sequence for the next read. PTY reads can split multi-byte characters.
fn flush_utf8(app: &AppHandle, id: &str, buffer: &mut Vec<u8>) {
    loop {
        match std::str::from_utf8(buffer) {
            Ok(text) => {
                emit_data(app, id, text.to_owned());
                buffer.clear();
                return;
            }
            Err(error) => {
                let valid = error.valid_up_to();
                if valid > 0 {
                    let text = std::str::from_utf8(&buffer[..valid])
                        .unwrap_or_default()
                        .to_owned();
                    emit_data(app, id, text);
                    buffer.drain(..valid);
                    continue;
                }
                if let Some(len) = error.error_len() {
                    buffer.drain(..len);
                    emit_data(app, id, "\u{FFFD}".to_owned());
                    continue;
                }
                // Incomplete trailing bytes: wait for the next chunk. Guard
                // against a PTY that never completes a bogus sequence.
                if buffer.len() > 64 * 1024 {
                    buffer.clear();
                }
                return;
            }
        }
    }
}

fn shell_command() -> CommandBuilder {
    #[cfg(windows)]
    {
        let mut command = CommandBuilder::new("powershell.exe");
        command.arg("-NoLogo");
        command
    }
    #[cfg(not(windows))]
    {
        let shell = std::env::var("SHELL")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "/bin/zsh".to_owned());
        let mut command = CommandBuilder::new(shell);
        command.arg("-l");
        command
    }
}

#[cfg(unix)]
fn kill_group(pid: Option<u32>) {
    if let Some(pid) = pid {
        // The shell starts a new session (setsid), so its process group id
        // equals its pid. Signal the group so children do not outlive the
        // panel. Ignore errors: the process may already be gone.
        unsafe {
            libc::kill(-(pid as i32), libc::SIGHUP);
        }
    }
}

#[cfg(not(unix))]
fn kill_group(_pid: Option<u32>) {}

#[tauri::command]
pub fn terminal_open(
    app: AppHandle,
    state: State<'_, Terminals>,
    cwd: Option<String>,
    cols: u16,
    rows: u16,
) -> Result<String, String> {
    let size = PtySize {
        rows: rows.max(2),
        cols: cols.max(2),
        pixel_width: 0,
        pixel_height: 0,
    };
    let pair = native_pty_system()
        .openpty(size)
        .map_err(|error| format!("open terminal: {error}"))?;

    let mut command = shell_command();
    if let Some(dir) = cwd.as_deref().filter(|path| !path.trim().is_empty()) {
        if std::path::Path::new(dir).is_dir() {
            command.cwd(dir);
        }
    }
    command.env("TERM", "xterm-256color");
    command.env("COLORTERM", "truecolor");

    let mut child = pair
        .slave
        .spawn_command(command)
        .map_err(|error| format!("start shell: {error}"))?;
    drop(pair.slave);

    let mut reader = pair
        .master
        .try_clone_reader()
        .map_err(|error| format!("read terminal: {error}"))?;
    let writer = pair
        .master
        .take_writer()
        .map_err(|error| format!("write terminal: {error}"))?;
    let killer = child.clone_killer();
    let pid = child.process_id();
    let id = uuid::Uuid::new_v4().to_string();

    let terminals = state.inner().clone();
    terminals.lock().insert(
        id.clone(),
        Session {
            writer,
            master: pair.master,
            killer,
            pid,
        },
    );

    let reader_app = app.clone();
    let reader_id = id.clone();
    thread::spawn(move || {
        let mut chunk = [0u8; 8192];
        let mut buffer = Vec::new();
        loop {
            match reader.read(&mut chunk) {
                Ok(0) => break,
                Ok(count) => {
                    buffer.extend_from_slice(&chunk[..count]);
                    flush_utf8(&reader_app, &reader_id, &mut buffer);
                }
                Err(_) => break,
            }
        }
    });

    let waiter_app = app.clone();
    let waiter_id = id.clone();
    let waiter_terminals = terminals.clone();
    thread::spawn(move || {
        let code = child.wait().ok().map(|status| status.exit_code());
        let _ = waiter_app.emit(
            "terminal:exit",
            TerminalExit {
                id: waiter_id.clone(),
                code,
            },
        );
        waiter_terminals.lock().remove(&waiter_id);
    });

    Ok(id)
}

#[tauri::command]
pub fn terminal_write(state: State<'_, Terminals>, id: String, data: String) -> Result<(), String> {
    let mut terminals = state.lock();
    let session = terminals
        .get_mut(&id)
        .ok_or_else(|| "terminal session is closed".to_owned())?;
    session
        .writer
        .write_all(data.as_bytes())
        .map_err(|error| format!("write terminal: {error}"))?;
    session
        .writer
        .flush()
        .map_err(|error| format!("write terminal: {error}"))
}

#[tauri::command]
pub fn terminal_resize(
    state: State<'_, Terminals>,
    id: String,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    let terminals = state.lock();
    let session = terminals
        .get(&id)
        .ok_or_else(|| "terminal session is closed".to_owned())?;
    session
        .master
        .resize(PtySize {
            rows: rows.max(2),
            cols: cols.max(2),
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|error| format!("resize terminal: {error}"))
}

#[tauri::command]
pub fn terminal_close(state: State<'_, Terminals>, id: String) {
    // Dropping the session sends the hangup and frees the master fd.
    let session = state.lock().remove(&id);
    drop(session);
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn pty_streams_shell_output() {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("open pty");
        let mut command = CommandBuilder::new("/bin/sh");
        command.arg("-c");
        command.arg("printf kodo-pty");
        let mut child = pair.slave.spawn_command(command).expect("spawn shell");
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader().expect("clone reader");
        let mut output = Vec::new();
        let mut chunk = [0u8; 1024];
        loop {
            match reader.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(count) => output.extend_from_slice(&chunk[..count]),
            }
        }

        assert!(child.wait().expect("wait for shell").success());
        assert!(String::from_utf8_lossy(&output).contains("kodo-pty"));
    }
}
