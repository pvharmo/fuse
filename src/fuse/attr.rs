use std::{ffi::OsStr};
use std::time::SystemTime;
use libc::ENOENT;

use fuser::{ReplyAttr, ReplyEntry, Request};

use super::{FuseFS, TTL, ROOT_DIR_ATTR};

impl FuseFS {
    pub fn internal_lookup(&mut self, _req: &Request, parent_inode: u64, name: &OsStr, reply: ReplyEntry) {
        println!("lookup: {parent_inode}, {}", name.to_str().unwrap());

        let mut node = self.tree.find_with_name(parent_inode, name.to_str().unwrap());

        if node.is_none() {
            if let Some(parent_node) = self.tree.find_with_inode(parent_inode) {
                if let Ok(mut parent_node) = parent_node.lock() {
                    self.get_children(&mut parent_node);
                    node = self.tree.find_with_name(parent_inode, name.to_str().unwrap());
                }
            }
        }
        
        if let Some(fs_node) = node {
            if let Ok(node) = fs_node.lock() {
                reply.entry(&TTL, &(*node).clone().into(), 0);
            }
        } else {
            reply.error(ENOENT);
        }
    }

    pub fn internal_setattr(
            &mut self,
            _req: &Request<'_>,
            ino: u64,
            _mode: Option<u32>,
            uid: Option<u32>,
            gid: Option<u32>,
            size: Option<u64>,
            atime: Option<fuser::TimeOrNow>,
            mtime: Option<fuser::TimeOrNow>,
            ctime: Option<std::time::SystemTime>,
            _fh: Option<u64>,
            crtime: Option<std::time::SystemTime>,
            _chgtime: Option<std::time::SystemTime>,
            _bkuptime: Option<std::time::SystemTime>,
            _flags: Option<u32>,
            reply: ReplyAttr,
        ) {
        println!("setattr: {}", ino);

        if let Some(fs_node) = self.tree.find_with_inode(ino) {
            if let Ok(node) = fs_node.lock() {
                if let Some(mut metadata) = node.metadata {
                    metadata.size = size.unwrap_or(metadata.size);
                    metadata.atime = match atime.unwrap_or(fuser::TimeOrNow::Now) {
                        fuser::TimeOrNow::SpecificTime(time) => time,
                        fuser::TimeOrNow::Now => SystemTime::now(),
                    };
                    metadata.mtime = match mtime.unwrap_or(fuser::TimeOrNow::Now) {
                        fuser::TimeOrNow::SpecificTime(time) => time,
                        fuser::TimeOrNow::Now => SystemTime::now(),
                    };
                    metadata.ctime = ctime.unwrap_or(metadata.ctime);
                    metadata.crtime = crtime.unwrap_or(metadata.crtime);
                    metadata.uid = uid.unwrap_or(metadata.uid);
                    metadata.gid = gid.unwrap_or(metadata.gid);
                } else {
                    reply.error(ENOENT);
                    return;
                }
                reply.attr(&TTL, &(*node).clone().into())
            } else {
                reply.error(ENOENT);
                return;
            };
        } else {
            reply.error(ENOENT);
        }
    }

    pub fn internal_getattr(&mut self, _req: &Request, ino: u64, reply: ReplyAttr) {
        println!("getattr: {}", ino);

        if ino == 1 {
            reply.attr(&TTL, &ROOT_DIR_ATTR);
            return;
        }

        if let Some(fs_node) = self.tree.find_with_inode(ino) {
            if let Ok(mut node) = fs_node.lock() {
                let provider = self.providers.get_provider(node.provider_id.as_ref().clone()).unwrap();

                let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    
                rt.block_on(async {
                    let metadata = provider.as_filesystem().unwrap().get_metadata(node.id.clone()).await.unwrap();

                    node.metadata = Some(metadata.into());
                });

                reply.attr(&TTL, &(*node).clone().into());
            }
        } else {
            reply.error(ENOENT);
        }
    }
}

#[cfg(test)]
mod attr_test {
    use super::*;
    use super::super::*;
    
    #[test]
    fn get_node() {
        
    }
}