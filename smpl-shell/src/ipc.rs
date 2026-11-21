use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

const MAX_REQUEST_BYTES: u64 = 128 * 1024;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "command", rename_all = "kebab-case")]
pub enum Request {
    Show,
    Hide,
    Toggle,
    Apply,
    ClosePopups,
    Popup { name: Popup },
    Identify,
    SetKeypadPresent { present: bool },
    KeypadSheet { sheet: serde_json::Value },
    TelemetrySample,
    Status,
    Quit,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Popup {
    QuickSettings,
    Usb,
    WorkspaceOverview,
    KeypadCheatsheet,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Response {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<serde_json::Value>,
}

impl Response {
    pub fn ok(status: Option<serde_json::Value>) -> Self {
        Self {
            ok: true,
            error: None,
            status,
        }
    }

    pub fn error(error: impl Into<String>) -> Self {
        Self {
            ok: false,
            error: Some(error.into()),
            status: None,
        }
    }
}

pub fn socket_path() -> Option<PathBuf> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            Some(PathBuf::from(format!("/run/user/{}", unsafe {
                libc::getuid()
            })))
        })?;
    Some(runtime.join("smplos/smpl-bar.sock"))
}

pub struct Server {
    listener: UnixListener,
    path: PathBuf,
}

impl Server {
    pub fn bind(path: &Path) -> std::io::Result<Self> {
        let directory = path.parent().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "IPC socket needs a parent",
            )
        })?;
        std::fs::create_dir_all(directory)?;
        std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))?;
        match UnixStream::connect(path) {
            Ok(_) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    "another smpl-bar already owns the IPC socket",
                ));
            }
            Err(_) => {
                let _ = std::fs::remove_file(path);
            }
        }
        let listener = UnixListener::bind(path)?;
        listener.set_nonblocking(true)?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        Ok(Self {
            listener,
            path: path.to_owned(),
        })
    }

    pub fn drain(&self) -> Vec<(Request, UnixStream)> {
        let mut requests = Vec::new();
        loop {
            let mut stream = match self.listener.accept() {
                Ok((stream, _)) => stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(error) => {
                    eprintln!("smpl-bar: IPC accept failed: {error}");
                    break;
                }
            };
            let _ = stream.set_nonblocking(false);
            let _ = stream.set_read_timeout(Some(Duration::from_millis(250)));
            let _ = stream.set_write_timeout(Some(Duration::from_millis(250)));
            let mut reader = BufReader::new((&mut stream).take(MAX_REQUEST_BYTES));
            let mut line = Vec::new();
            match reader.read_until(b'\n', &mut line) {
                Ok(0) | Err(_) => {
                    let _ = write_response(
                        &mut stream,
                        &Response::error("empty or unreadable request"),
                    );
                }
                Ok(_) => match serde_json::from_slice(&line) {
                    Ok(request) => requests.push((request, stream)),
                    Err(error) => {
                        let _ = write_response(
                            &mut stream,
                            &Response::error(format!("invalid request: {error}")),
                        );
                    }
                },
            }
        }
        requests
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

pub fn send(path: &Path, request: &Request) -> std::io::Result<Response> {
    let mut stream = UnixStream::connect(path)?;
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    serde_json::to_writer(&mut stream, request)?;
    stream.write_all(b"\n")?;
    stream.shutdown(std::net::Shutdown::Write)?;
    let mut response_bytes = Vec::new();
    let mut buffer = [0_u8; 4096];
    loop {
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => {
                response_bytes.extend_from_slice(&buffer[..count]);
                if count >= 1 && buffer[count - 1] == b'\n' {
                    break;
                }
                if response_bytes.len() > MAX_REQUEST_BYTES as usize {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "IPC response exceeded 128 KiB",
                    ));
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                ) =>
            {
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(error) => return Err(error),
        }
    }
    let response: Response = serde_json::from_slice(&response_bytes)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    Ok(response)
}

pub fn write_response(stream: &mut UnixStream, response: &Response) -> std::io::Result<()> {
    serde_json::to_writer(&mut *stream, response)?;
    stream.write_all(b"\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixStream;
    use std::thread;

    #[test]
    fn typed_request_round_trips_over_the_socket() {
        let fixture = tempfile::tempdir().unwrap();
        let path = fixture.path().join("bar.sock");
        let server = Server::bind(&path).unwrap();
        let client = thread::spawn(move || send(&path, &Request::Popup { name: Popup::Usb }));
        std::thread::sleep(Duration::from_millis(10));
        let mut requests = server.drain();
        assert_eq!(requests.len(), 1);
        let (request, mut stream) = requests.pop().unwrap();
        assert_eq!(request, Request::Popup { name: Popup::Usb });
        write_response(&mut stream, &Response::ok(None)).unwrap();
        assert!(client.join().unwrap().unwrap().ok);
    }

    #[test]
    fn malformed_request_gets_an_error_response() {
        let fixture = tempfile::tempdir().unwrap();
        let path = fixture.path().join("bar.sock");
        let server = Server::bind(&path).unwrap();
        let writer = thread::spawn(move || {
            let mut stream = UnixStream::connect(path).unwrap();
            stream.write_all(b"not-json\n").unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).unwrap();
            response
        });
        std::thread::sleep(Duration::from_millis(10));
        assert!(server.drain().is_empty());
        assert!(writer.join().unwrap().contains("\"ok\":false"));
    }
}
