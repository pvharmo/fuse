// create a wrapper to handle mounting and unmounting the filesystem
// Path: src/mount.rs
use std::path::Path;

use fuser::{MountOption, Filesystem};

pub struct Mount {
    mountpoint: String,
}

impl Mount {
    pub fn new<P: AsRef<Path>>(mountpoint: P) -> Self {
        Self {
            mountpoint: mountpoint.as_ref().to_str().unwrap().to_string(),
        }
    }

    pub fn mount<F: Filesystem + Send + Sync + 'static>(&self, fs: F) -> std::io::Result<()> {
        fuser::mount2(fs, &self.mountpoint, &[
            MountOption::AutoUnmount,
            MountOption::FSName(String::from("rust-fuse"))
        ])
    }
}