// src/index/mod.rs

pub(crate) mod code;
pub mod command;
pub(crate) mod config;
pub(crate) mod drift;
pub(crate) mod embed;
pub mod error;
pub(crate) mod git;
pub(crate) mod hash;
pub(crate) mod index;
pub(crate) mod k8s;
pub(crate) mod render;
pub(crate) mod types;

pub use command::Command;
pub use command::JsonCommand;
pub use command::RunCommand;
pub use error::Error;
pub use types::SearchQuery;

#[cfg(test)]
pub(crate) mod test_util {
    use std::sync::atomic::AtomicUsize;
    use std::sync::atomic::Ordering;

    use camino::Utf8Path;
    use camino::Utf8PathBuf;

    static TEST_DIR_COUNTER: AtomicUsize = AtomicUsize::new(0);

    pub(crate) struct TempDir(Utf8PathBuf);

    impl TempDir {
        pub(crate) fn new(label: &str) -> Self {
            let id = TEST_DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir = Utf8PathBuf::try_from(std::env::temp_dir())
                .expect("UTF-8 temp dir")
                .join(format!("blite-test-{label}-{}-{id}", std::process::id()));
            let _ = fs_err::remove_dir_all(&dir);
            fs_err::create_dir_all(&dir).expect("create temp test dir");
            Self(dir)
        }

        pub(crate) fn path(&self) -> &Utf8Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs_err::remove_dir_all(&self.0);
        }
    }
}
