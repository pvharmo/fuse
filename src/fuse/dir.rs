use std::{ffi::OsStr};
use std::os::unix::ffi::OsStrExt;
use std::time::SystemTime;
use libc::ENOENT;

use fuser::{FileType, FileAttr, ReplyDirectory, ReplyEntry, Request};
use crossroads::interfaces::filesystem::{ObjectId, File, Metadata as CrossroadsMetadata};

use super::{FuseFS, TTL};

impl FuseFS {
    pub fn internal_readdir(&mut self, _req: &Request, dir_inode: u64, _fh: u64, offset: i64, mut reply: ReplyDirectory) {
        println!("readdir: {}", dir_inode);

        if dir_inode == 1 {
            let providers = self.providers.list_providers();
            if offset < providers.len().try_into().unwrap() {
                let provider = providers.get(offset as usize).unwrap();
                let provider_name = provider.id.clone();
                let provider_name = provider_name.as_bytes();
                let node = self.tree.find_with_ids(ObjectId::root(), (*provider).clone());
                if let Ok(node) = node.unwrap().lock() {
                    let _ = reply.add(node.inode, offset + 1, FileType::Directory, OsStr::from_bytes(provider_name));
                }
            }
            reply.ok();
            return;
        }

        match offset {
            0 => {let _ = reply.add(1, 1, FileType::Directory, OsStr::from_bytes(b"."));},
            1 => {let _ = reply.add(1, 2, FileType::Directory, OsStr::from_bytes(b".."));},
            _ => {
                println!("offset: {}", offset);

                if let Some(fs_node) = self.tree.find_with_inode(dir_inode) {
                    let children = self.get_children(&mut fs_node.lock().unwrap());
                    if offset - 2 < children.len().try_into().unwrap() {
                        let child = children.get((offset) as usize - 2).unwrap().as_ref();
                        if let Ok(child) = child.lock() {
                            let file_name = child.name.clone();
                            let file_name = file_name.as_bytes();
                            let file_type = if child.id.is_directory() {
                                FileType::Directory
                            } else {
                                FileType::RegularFile
                            };
                            let _ = reply.add(child.inode, offset + 1, file_type, OsStr::from_bytes(file_name));
                        }
                    }
                }
            }
        }

        reply.ok();
    }

    pub fn internal_rmdir(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: fuser::ReplyEmpty) {
        println!("rmdir: {}", name.to_str().unwrap());

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

    pub fn internal_mkdir(
        &mut self,
        _req: &Request<'_>,
        parent: u64,
        name: &OsStr,
        _mode: u32,
        _umask: u32,
        reply: ReplyEntry,
    ) {
        println!("mkdir: {}", name.to_str().unwrap());

        if let Some(parent_dir) = self.tree.find_with_inode(parent) {
            if let Ok(mut parent_dir) = parent_dir.lock() {
                let provider = self.providers.get_provider(parent_dir.provider_id.as_ref().clone()).unwrap();
                let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    
                rt.block_on(async {
                    let id = ObjectId::directory(parent_dir.id.to_string() + "/" + name.to_str().unwrap());
                    provider.as_filesystem().unwrap().create(parent_dir.id.clone(), File {
                        id: id.clone(),
                        name: name.to_str().unwrap().to_string(),
                        metadata: Some(CrossroadsMetadata {
                            mime_type: Some("directory".to_string()),
                            created_at: None,
                            modified_at: None,
                            meta_changed_at: None,
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
                        kind: FileType::Directory,
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
}