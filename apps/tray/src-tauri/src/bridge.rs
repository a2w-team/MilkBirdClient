use serde::{Deserialize, Serialize};
use std::env;
use std::error::Error;
use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::time::Duration;

const MAX_MESSAGE: u64 = 8192;
const ENDPOINT: &str = "api/v1/announcement-lookups";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Query {
    campus: String,
    topic: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Submission {
    version: u8,
    confirmed: bool,
    query: Query,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Announcement {
    id: String,
    campus: String,
    topic: String,
    title: String,
    summary: String,
    source: String,
    source_url: String,
    published_at: String,
    valid_until: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum LookupResult {
    Matched {
        announcement: Announcement,
    },
    NeedsClarification {
        #[serde(rename = "missingFields")]
        missing_fields: Vec<String>,
    },
    NoMatch,
    Error {
        code: String,
    },
}

fn failure(code: &str) -> LookupResult {
    LookupResult::Error { code: code.into() }
}

struct Bridge {
    listener: UnixListener,
    path: PathBuf,
    _lock: File,
}

impl Drop for Bridge {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn bind() -> Result<Bridge, Box<dyn Error>> {
    let dir = env::var_os("MILKBIRD_TRAY_DIR")
        .map(PathBuf::from)
        .unwrap_or(PathBuf::from(env::var_os("HOME").ok_or("HOME is required")?).join(".milkbird"));
    match DirBuilder::new().mode(0o700).create(&dir) {
        Ok(()) => {}
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {}
        Err(err) => return Err(err.into()),
    }
    let metadata = fs::symlink_metadata(&dir)?;
    if !metadata.is_dir() || metadata.permissions().mode() & 0o077 != 0 {
        return Err("tray socket directory must be private (0700) and not a symlink".into());
    }

    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(dir.join("tray.lock"))?;
    lock.try_lock()?;
    let path = dir.join("tray.sock");
    match fs::symlink_metadata(&path) {
        Ok(meta) if meta.file_type().is_socket() => fs::remove_file(&path)?,
        Ok(_) => return Err("tray.sock exists and is not a socket".into()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => return Err(err.into()),
    }
    let listener = UnixListener::bind(&path)?;
    let bridge = Bridge {
        listener,
        path,
        _lock: lock,
    };
    fs::set_permissions(&bridge.path, fs::Permissions::from_mode(0o600))?;
    Ok(bridge)
}

fn submit(
    stream: &mut UnixStream,
    client: &reqwest::blocking::Client,
    url: &reqwest::Url,
) -> Option<LookupResult> {
    let mut line = Vec::new();
    let mut reader = BufReader::new((&*stream).take(MAX_MESSAGE + 1));
    reader.read_until(b'\n', &mut line).ok()?;
    if line.len() as u64 > MAX_MESSAGE || line.last() != Some(&b'\n') {
        return Some(failure("invalid_request"));
    }
    let Ok(frame) = serde_json::from_slice::<Submission>(&line) else {
        return Some(failure("invalid_request"));
    };
    if frame.version != 1 || !frame.confirmed {
        return Some(failure("confirmation_required"));
    }

    let response = match client.post(url.clone()).json(&frame.query).send() {
        Ok(response) => response,
        Err(_) => return Some(failure("server_unavailable")),
    };
    if response.status() == reqwest::StatusCode::BAD_REQUEST {
        return Some(failure("invalid_request"));
    }
    if !response.status().is_success() {
        return Some(failure("server_unavailable"));
    }
    let mut data = Vec::new();
    if response
        .take(MAX_MESSAGE + 1)
        .read_to_end(&mut data)
        .is_err()
        || data.len() as u64 > MAX_MESSAGE
    {
        return Some(failure("invalid_response"));
    }
    Some(serde_json::from_slice(&data).unwrap_or_else(|_| failure("invalid_response")))
}

pub fn start(publish: impl Fn(LookupResult) + Send + 'static) -> Result<(), Box<dyn Error>> {
    let origin = env::var("MILKBIRD_API_ORIGIN").unwrap_or_else(|_| "http://127.0.0.1:8080".into());
    let base = reqwest::Url::parse(&origin)?;
    let loopback = base.host_str() == Some("127.0.0.1") || base.host_str() == Some("localhost");
    if (base.scheme() != "https" && !(base.scheme() == "http" && loopback))
        || base.path() != "/"
        || base.query().is_some()
        || base.fragment().is_some()
        || !base.username().is_empty()
        || base.password().is_some()
    {
        return Err("MILKBIRD_API_ORIGIN must be an HTTPS origin or loopback HTTP origin".into());
    }
    let url = base.join(ENDPOINT)?;
    let bridge = bind()?;
    std::thread::Builder::new().name("tray-bridge".into()).spawn(move || {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(5))
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .build()
            .expect("HTTP client");
        for stream in bridge.listener.incoming() {
            let Ok(mut stream) = stream else { break };
            if stream.set_read_timeout(Some(Duration::from_secs(2))).is_err() ||
                stream.set_write_timeout(Some(Duration::from_secs(2))).is_err()
            {
                continue;
            }
            if let Some(result) = submit(&mut stream, &client, &url) {
                // An invalid local frame must not open or overwrite the result window.
                if !matches!(&result, LookupResult::Error { code } if code == "invalid_request" || code == "confirmation_required") {
                    publish(result.clone());
                }
                if serde_json::to_writer(&mut stream, &result).is_ok() {
                    let _ = stream.write_all(b"\n");
                }
            }
        }
    })?;
    Ok(())
}
