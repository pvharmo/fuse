use std::{ffi::OsStr};
use std::time::SystemTime;
use libc::ENOENT;
use chrono;

use fuser::{FileType, FileAttr, ReplyData, ReplyEntry, Request};
use crossroads::interfaces::filesystem::{ObjectId, File, Metadata as CrossroadsMetadata};

use crate::fstree::FileState;
use super::{FuseFS, TTL};

impl FuseFS {
    pub fn internal_unlink(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: fuser::ReplyEmpty) {
        println!("unlink: {}", name.to_str().unwrap());

        if let Some(node) = self.tree.find_with_name(parent, name.to_str().unwrap()) {
            if let Ok(node) = node.lock() {
                let provider = self.providers.get_provider(node.provider_id.as_ref().clone()).unwrap();
                let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    
                rt.block_on(async {
                    provider.as_filesystem().unwrap().delete(node.id.clone()).await.unwrap();
                });
            }

            if let Some(parent_node) = self.tree.find_with_inode(parent) {
                if let Ok(mut parent_node) = parent_node.lock() {
                    parent_node.children.retain(|child| child.lock().unwrap().name != name.to_str().unwrap());
                }
            }

            self.tree.remove(parent, node);
        }

        reply.ok();
    }

    pub fn internal_mknod(
        &mut self,
        _req: &Request<'_>,
        parent: u64,
        name: &OsStr,
        _mode: u32,
        _umask: u32,
        _rdev: u32,
        reply: ReplyEntry,
    ) {
        println!("mknod: {}", name.to_str().unwrap());

        if let Some(parent_dir) = self.tree.find_with_inode(parent) {
            if let Ok(mut parent_dir) = parent_dir.lock() {
                let provider = self.providers.get_provider(parent_dir.provider_id.as_ref().clone()).unwrap();
                let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    
                rt.block_on(async {
                    let id = ObjectId::new(parent_dir.id.to_string() + "/" + name.to_str().unwrap(), crossroads::interfaces::filesystem::FileType::File);
                    provider.as_filesystem().unwrap().create(parent_dir.id.clone(), File {
                        id: id.clone(),
                        name: name.to_str().unwrap().to_string(),
                        metadata: Some(CrossroadsMetadata {
                            mime_type: None,
                            created_at: Some(chrono::Utc::now()),
                            modified_at: Some(chrono::Utc::now()),
                            meta_changed_at: Some(chrono::Utc::now()),
                            accessed_at: None,
                            size: None,
                            open_path: None,
                            owner: None,
                            permissions: None,
                        }),
                    }).await.unwrap();

                    let provider_id = parent_dir.provider_id.clone();

                    let new_file = self.tree.new_file(&mut parent_dir, id, name.to_str().unwrap(), None, provider_id);

                    reply.entry(&TTL, &FileAttr {
                        ino: new_file.lock().unwrap().inode,
                        size: 0,
                        blocks: 0,
                        atime: SystemTime::now(), // 1970-01-01 00:00:00
                        mtime: SystemTime::now(),
                        ctime: SystemTime::now(),
                        crtime: SystemTime::now(),
                        kind: FileType::RegularFile,
                        perm: 0o755,
                        nlink: 0,
                        uid: 501,
                        gid: 20,
                        rdev: 0,
                        flags: 0,
                        blksize: 512,                    
                    }, 0);
                });
            }
        } else {
            reply.error(ENOENT);
        }
    }
    
    pub fn internal_read(&mut self, _req: &Request, ino: u64, _fh: u64, offset: i64, size: u32, _flags: i32, _lock_owner: Option<u64>, reply: ReplyData) {
        println!("read: {}", ino);

        if let Some(file) = self.tree.find_with_inode(ino) {
            if let Ok(file) = file.lock() {
                let provider = self.providers.get_provider(file.provider_id.as_ref().clone()).unwrap();
                let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    
                rt.block_on(async {
                    let data = provider.as_filesystem().unwrap().read_file(file.id.clone()).await.unwrap();
                    println!("--- read {} offset: {offset}, size: {size} ---", file.id.as_str());
                    reply.data(&data[offset as usize..std::cmp::min(offset as usize + size as usize, data.len())]);
                });
            }
        } else {
            reply.error(ENOENT);
        }
    }

    pub fn internal_rename(
            &mut self,
            _req: &Request<'_>,
            parent: u64,
            name: &OsStr,
            newparent: u64,
            newname: &OsStr,
            _flags: u32,
            reply: fuser::ReplyEmpty,
        ) {
        println!("rename: {} -> {}", name.to_str().unwrap(), newname.to_str().unwrap());

        if let Some(node) = self.tree.find_with_name(parent, name.to_str().unwrap()) {
            if let Ok(mut node) = node.lock() {
                let provider = self.providers.get_provider(node.provider_id.as_ref().clone()).unwrap();
                let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    
                rt.block_on(async {
                    let mut object_id = node.id.clone();
                    if name != newname {
                        object_id = provider.as_filesystem().unwrap().rename(node.id.clone(), newname.to_str().unwrap().to_string()).await.unwrap();
                        node.name = newname.to_str().unwrap().to_string();
                        self.tree.rename(parent, name.to_str().unwrap(), newname.to_str().unwrap());
                    }

                    if parent != newparent {
                        let new_parent = self.tree.find_with_inode(newparent).unwrap();
                        let new_parent = new_parent.lock().unwrap();
                        object_id = provider.as_filesystem().unwrap().move_to(object_id.clone(), new_parent.id.clone()).await.unwrap();
                    }

                    node.children = Vec::new();
                    node.content_state = FileState::ShallowReady;

                    node.id = object_id;

                    reply.ok();
                });
            }
        } else {
            reply.error(ENOENT);
        }
    }

    pub fn internal_open(&mut self, _req: &Request<'_>, _ino: u64, _flags: i32, reply: fuser::ReplyOpen) {
        println!("open: {}", _ino);

        reply.opened(0, 0)
    }

    pub fn internal_write(
            &mut self,
            _req: &Request<'_>,
            ino: u64,
            _fh: u64,
            offset: i64,
            data: &[u8],
            _write_flags: u32,
            _flags: i32,
            _lock_owner: Option<u64>,
            reply: fuser::ReplyWrite,
        ) {
        println!("write: {}", ino);

        if let Some(file) = self.tree.find_with_inode(ino) {
            if let Ok(mut file) = file.lock() {
                let provider = self.providers.get_provider(file.provider_id.as_ref().clone()).unwrap();
                let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    
                rt.block_on(async {
                    let file_content = provider.as_filesystem().unwrap().read_file(file.id.clone()).await.unwrap();
                    let content;
                    if offset > 0 {
                        content = [&file_content[0..offset as usize], data].concat();
                    } else {
                        content = data.to_vec();
                    }
                    provider.as_filesystem().unwrap().write_file(file.id.clone(), content.into()).await.unwrap();
                    let metadata = file.metadata.as_mut().unwrap();
                    metadata.size = data.len() as u64;
                    reply.written(data.len() as u32);
                });
            }
        } else {
            reply.error(ENOENT);
        }
    }
}