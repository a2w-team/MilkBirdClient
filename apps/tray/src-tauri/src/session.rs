use serde::{Deserialize, Serialize};
use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Member {
    pub id: String,
    pub email: String,
    pub display_name: String,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct Session {
    pub token: String,
    pub member: Member,
}

impl Session {
    pub fn is_valid(&self) -> bool {
        !self.token.is_empty() && !self.member.id.is_empty() && !self.member.email.is_empty()
    }
}

pub fn directory() -> io::Result<PathBuf> {
    if let Some(dir) = std::env::var_os("MILKBIRD_TRAY_DIR").filter(|dir| !dir.is_empty()) {
        return Ok(dir.into());
    }
    let home = std::env::var_os("HOME").ok_or_else(|| io::Error::other("HOME is required"))?;
    Ok(PathBuf::from(home).join(".milkbird"))
}

pub fn ensure_private_dir(dir: &Path) -> io::Result<()> {
    match DirBuilder::new().mode(0o700).create(dir) {
        Ok(()) => fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?,
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {}
        Err(err) => return Err(err),
    }
    let meta = fs::symlink_metadata(dir)?;
    if !meta.is_dir() || meta.permissions().mode() & 0o777 != 0o700 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "tray directory must be a directory with mode 0700",
        ));
    }
    Ok(())
}

pub struct SessionStore {
    path: PathBuf,
    session: Mutex<Option<Session>>,
}

impl SessionStore {
    pub fn open(dir: &Path) -> io::Result<Self> {
        ensure_private_dir(dir)?;
        let path = dir.join("session.json");
        let session = match fs::symlink_metadata(&path) {
            Ok(meta) => {
                if !meta.is_file() || meta.permissions().mode() & 0o777 != 0o600 {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "session.json must be a regular file with mode 0600",
                    ));
                }
                let session: Session = serde_json::from_reader(File::open(&path)?)?;
                if !session.is_valid() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "invalid session",
                    ));
                }
                Some(session)
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => None,
            Err(err) => return Err(err),
        };
        Ok(Self {
            path,
            session: Mutex::new(session),
        })
    }

    pub fn get(&self) -> Option<Session> {
        self.session.lock().expect("session store").clone()
    }

    pub fn set(&self, session: Session) -> io::Result<()> {
        let mut current = self.session.lock().expect("session store");
        write_private_json(&self.path, &session)?;
        *current = Some(session);
        Ok(())
    }

    pub fn clear(&self) -> io::Result<()> {
        let mut current = self.session.lock().expect("session store");
        match fs::remove_file(&self.path) {
            Ok(()) => {}
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => return Err(err),
        }
        *current = None;
        Ok(())
    }
}

pub fn write_private_json(path: &Path, value: &impl Serialize) -> io::Result<()> {
    static NEXT_FILE: AtomicU64 = AtomicU64::new(0);
    let (temporary, mut file) = loop {
        let sequence = NEXT_FILE.fetch_add(1, Ordering::Relaxed);
        let name = format!(".tray-{}-{sequence}.tmp", std::process::id());
        let temporary = path.with_file_name(name);
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)
        {
            Ok(file) => break (temporary, file),
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err),
        }
    };

    let result = (|| {
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
        serde_json::to_writer(&mut file, value)?;
        file.sync_all()?;
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

#[cfg(test)]
pub mod tests {
    use super::*;

    pub struct TestDir(PathBuf);

    impl TestDir {
        pub fn new() -> Self {
            static NEXT_DIR: AtomicU64 = AtomicU64::new(0);
            let seq = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
            let path =
                std::env::temp_dir().join(format!("milkbird-test-{}-{seq}", std::process::id()));
            ensure_private_dir(&path).unwrap();
            Self(path)
        }

        pub fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn session_round_trip_and_permissions() {
        let dir = TestDir::new();
        let store = SessionStore::open(dir.path()).unwrap();
        assert!(store.get().is_none());
        let mut session = Session {
            token: "first".into(),
            member: Member {
                id: "m1".into(),
                email: "student@example.edu".into(),
                display_name: "奶鸟".into(),
            },
        };
        store.set(session.clone()).unwrap();
        session.token = "second".into();
        store.set(session).unwrap();
        let path = dir.path().join("session.json");
        assert_eq!(
            fs::metadata(dir.path()).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
        let reopened = SessionStore::open(dir.path()).unwrap();
        let loaded = reopened.get().unwrap();
        assert_eq!(loaded.token, "second");
        assert_eq!(loaded.member.display_name, "奶鸟");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(SessionStore::open(dir.path()).is_err());
        reopened.clear().unwrap();
        assert!(!path.exists());
        assert!(reopened.get().is_none());
        assert!(SessionStore::open(dir.path()).unwrap().get().is_none());
    }
}
