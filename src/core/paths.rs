use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

use fs2::FileExt;

use crate::Result;

/// Where Runspace keeps its transient state.
///
/// The runtime root is chosen to be RAM-backed when possible and always
/// per-user. Everything below it disappears when the last runspace is
/// destroyed (apart from the zero-byte lock file used for serialisation).
#[derive(Debug, Clone)]
pub struct Paths {
    root: PathBuf,
}

impl Paths {
    pub fn discover() -> Result<Self> {
        if let Some(dir) = std::env::var_os("RUNSPACE_RUNTIME_DIR") {
            let root = PathBuf::from(dir);
            if root.as_os_str().is_empty() {
                return Err(crate::Error::invalid(
                    "RUNSPACE_RUNTIME_DIR is set but empty",
                ));
            }
            return Ok(Self { root });
        }

        #[cfg(target_os = "linux")]
        {
            if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR")
                && !dir.is_empty()
            {
                return Ok(Self {
                    root: PathBuf::from(dir).join("runspace"),
                });
            }

            let uid = unsafe { libc::getuid() };
            let shm = Path::new("/dev/shm");
            let root = if is_writable_dir(shm) {
                shm.join(format!("runspace-{uid}"))
            } else {
                std::env::temp_dir().join(format!("runspace-{uid}"))
            };
            Ok(Self { root })
        }

        #[cfg(windows)]
        {
            Ok(Self {
                root: std::env::temp_dir().join("runspace"),
            })
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn ensure(&self) -> Result<()> {
        fs::create_dir_all(self.runspaces_dir())?;
        set_private_dir(&self.root)?;
        set_private_dir(&self.runspaces_dir())?;
        Ok(())
    }

    pub fn runspaces_dir(&self) -> PathBuf {
        self.root.join("runspaces")
    }

    pub fn dir(&self, id: &str) -> PathBuf {
        self.runspaces_dir().join(id)
    }

    pub fn record_path(&self, id: &str) -> PathBuf {
        self.dir(id).join("state.json")
    }

    pub fn log_path(&self, id: &str) -> PathBuf {
        self.dir(id).join("runspace.log")
    }

    pub fn activity_path(&self, id: &str) -> PathBuf {
        self.dir(id).join("activity")
    }

    pub fn home_dir(&self, id: &str) -> PathBuf {
        self.dir(id).join("home")
    }

    pub fn tmp_dir(&self, id: &str) -> PathBuf {
        self.dir(id).join("tmp")
    }

    pub fn xdg_dir(&self, id: &str) -> PathBuf {
        self.dir(id).join("runtime")
    }

    /// Serialise lifecycle changes between Runspace processes. Blocking; the
    /// critical sections are short.
    pub fn lock(&self) -> Result<Lock> {
        self.ensure()?;
        let mut options = OpenOptions::new();
        options.create(true).truncate(false).read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(self.root.join("lock"))?;
        file.lock_exclusive()?;
        Ok(Lock { file })
    }
}

pub struct Lock {
    file: File,
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = fs2::FileExt::unlock(&self.file);
    }
}

#[cfg(unix)]
fn set_private_dir(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = fs::metadata(path)?.permissions();
    if perms.mode() & 0o777 != 0o700 {
        perms.set_mode(0o700);
        fs::set_permissions(path, perms)?;
    }
    Ok(())
}

#[cfg(windows)]
fn set_private_dir(_path: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn is_writable_dir(path: &Path) -> bool {
    use std::ffi::CString;
    if !path.is_dir() {
        return false;
    }
    let Ok(path) = CString::new(path.as_os_str().as_encoded_bytes()) else {
        return false;
    };
    unsafe { libc::access(path.as_ptr(), libc::W_OK) == 0 }
}
