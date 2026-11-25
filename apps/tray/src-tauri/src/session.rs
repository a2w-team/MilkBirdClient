use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::collections::{BTreeMap, HashSet};
use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::{self, BufReader};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

const NOTIFICATION_STATE_FILE: &str = "notification-state.json";

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

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub(crate) struct NotificationState {
    pub(crate) last_seq: u64,
    pub(crate) demand_response_unread: BTreeMap<String, u64>,
    pub(crate) unread_by_room: BTreeMap<String, u64>,
    pub(crate) room_to_demand: BTreeMap<String, String>,
    pub(crate) seen_notices: HashSet<(String, String)>,
}

type NotificationStates = BTreeMap<String, BTreeMap<String, NotificationState>>;

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
    notification_write: Mutex<()>,
}

impl SessionStore {
    pub fn open(dir: &Path) -> io::Result<Self> {
        ensure_private_dir(dir)?;
        let path = dir.join("session.json");
        let session = read_private_json::<Session>(&path)?;
        if session.as_ref().is_some_and(|session| !session.is_valid()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid session",
            ));
        }
        Ok(Self {
            path,
            session: Mutex::new(session),
            notification_write: Mutex::new(()),
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

    pub(crate) fn load_notification_state(
        &self,
        origin: &str,
        member_id: &str,
    ) -> Option<NotificationState> {
        let path = self.path.with_file_name(NOTIFICATION_STATE_FILE);
        let states = read_notification_states(&path).ok()?;
        states.get(origin)?.get(member_id).cloned()
    }

    pub(crate) fn save_notification_state(
        &self,
        origin: &str,
        member_id: &str,
        state: &NotificationState,
    ) -> io::Result<()> {
        let _guard = self.notification_write.lock().expect("notification state");
        let path = self.path.with_file_name(NOTIFICATION_STATE_FILE);
        let mut states = read_notification_states(&path)?;
        let origin_states = states.entry(origin.into()).or_default();
        if origin_states.get(member_id) == Some(state) {
            return Ok(());
        }
        origin_states.insert(member_id.into(), state.clone());
        write_private_json(&path, &states)
    }
}

fn read_notification_states(path: &Path) -> io::Result<NotificationStates> {
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(err) => return Err(err),
    };
    if !meta.is_file() || meta.permissions().mode() & 0o777 != 0o600 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "notification-state.json must be a regular file with mode 0600",
        ));
    }
    match serde_json::from_reader(BufReader::new(File::open(path)?)) {
        Ok(states) => Ok(states),
        Err(err) if !err.is_io() => Ok(BTreeMap::new()),
        Err(err) => Err(err.into()),
    }
}

pub(crate) fn write_private_json(path: &Path, value: &impl Serialize) -> io::Result<()> {
    static NEXT_FILE: AtomicU64 = AtomicU64::new(0);
    let (temporary, mut file) = loop {
        let sequence = NEXT_FILE.fetch_add(1, Ordering::Relaxed);
        let name = format!(".tray-state-{}-{sequence}.tmp", std::process::id());
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

pub fn read_private_json<T: DeserializeOwned>(path: &Path) -> io::Result<Option<T>> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_file() && meta.permissions().mode() & 0o777 == 0o600 => {
            Ok(Some(serde_json::from_reader(File::open(path)?)?))
        }
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "tray data must be a regular file with mode 0600",
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::os::unix::fs::{MetadataExt, symlink};

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

    fn notification_state(last_seq: u64) -> NotificationState {
        NotificationState {
            last_seq,
            demand_response_unread: BTreeMap::from([("d1".into(), 2)]),
            unread_by_room: BTreeMap::from([("r1".into(), 3)]),
            room_to_demand: BTreeMap::from([("r1".into(), "d1".into())]),
            seen_notices: HashSet::from([("room.ready".into(), "r1".into())]),
        }
    }

    #[test]
    fn notification_state_survives_restart_and_is_scoped_by_origin_and_member() {
        let dir = TestDir::new();
        let path = dir.path().join(NOTIFICATION_STATE_FILE);
        let store = SessionStore::open(dir.path()).unwrap();
        let origin = "https://club.example";
        let other_origin = "https://test.example";
        let first = notification_state(42);
        let second = notification_state(81);
        let third = notification_state(7);
        assert!(store.load_notification_state(origin, "m1").is_none());

        store.save_notification_state(origin, "m1", &first).unwrap();
        let first_file = fs::metadata(&path).unwrap();
        store.save_notification_state(origin, "m1", &first).unwrap();
        assert_eq!(fs::metadata(&path).unwrap().ino(), first_file.ino());
        store.save_notification_state(origin, "m2", &second).unwrap();
        store.save_notification_state(other_origin, "m1", &third).unwrap();
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        drop(store);

        let reopened = SessionStore::open(dir.path()).unwrap();
        assert_eq!(reopened.load_notification_state(origin, "m1"), Some(first));
        assert_eq!(reopened.load_notification_state(origin, "m2"), Some(second));
        assert_eq!(reopened.load_notification_state(other_origin, "m1"), Some(third));
        assert!(reopened.load_notification_state(other_origin, "m2").is_none());
    }

    #[test]
    fn malformed_notification_state_leaves_legacy_session_valid_and_can_be_replaced() {
        let dir = TestDir::new();
        let session_path = dir.path().join("session.json");
        let sidecar_path = dir.path().join(NOTIFICATION_STATE_FILE);
        let login = serde_json::json!({
            "token": "synthetic-private-token",
            "member": {"id": "m1", "email": "student@example.edu", "displayName": "奶鸟"}
        });
        write_private_json(&session_path, &login).unwrap();
        let original_session = fs::read(&session_path).unwrap();
        fs::write(&sidecar_path, b"{").unwrap();
        fs::set_permissions(&sidecar_path, fs::Permissions::from_mode(0o600)).unwrap();

        let store = SessionStore::open(dir.path()).unwrap();
        let origin = "https://club.example";
        assert_eq!(store.get().unwrap().token, "synthetic-private-token");
        assert!(store.load_notification_state(origin, "m1").is_none());
        assert_eq!(fs::read(&sidecar_path).unwrap(), b"{");

        let state = notification_state(2);
        store.save_notification_state(origin, "m1", &state).unwrap();
        assert_eq!(fs::read(&session_path).unwrap(), original_session);
        assert!(!fs::read_to_string(&sidecar_path).unwrap().contains("synthetic-private-token"));
        store.clear().unwrap();
        let reopened = SessionStore::open(dir.path()).unwrap();
        assert!(reopened.get().is_none());
        assert_eq!(reopened.load_notification_state(origin, "m1"), Some(state));
    }

    #[test]
    fn notification_state_rejects_symlinks_and_unsafe_permissions() {
        let dir = TestDir::new();
        let path = dir.path().join(NOTIFICATION_STATE_FILE);
        let linked_path = dir.path().join("unrelated.json");
        let store = SessionStore::open(dir.path()).unwrap();
        let state = notification_state(3);
        let origin = "https://club.example";
        write_private_json(&linked_path, &serde_json::json!({"unrelated": true})).unwrap();
        symlink(&linked_path, &path).unwrap();

        assert!(store.load_notification_state(origin, "m1").is_none());
        let error = store.save_notification_state(origin, "m1", &state).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(fs::read_link(&path).unwrap(), linked_path);
        fs::remove_file(&path).unwrap();

        store.save_notification_state(origin, "m1", &state).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(store.load_notification_state(origin, "m1").is_none());
        let error = store
            .save_notification_state(origin, "m1", &notification_state(5))
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(store.load_notification_state(origin, "m1"), Some(state));
    }

    #[test]
    fn failed_atomic_write_preserves_the_original_and_cleans_up() {
        struct FailingValue;

        impl Serialize for FailingValue {
            fn serialize<S: serde::Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
                Err(serde::ser::Error::custom("intentional serialization error"))
            }
        }

        let dir = TestDir::new();
        let path = dir.path().join(NOTIFICATION_STATE_FILE);
        write_private_json(&path, &notification_state(3)).unwrap();
        let original = fs::read(&path).unwrap();

        assert!(write_private_json(&path, &FailingValue).is_err());
        assert_eq!(fs::read(&path).unwrap(), original);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}
