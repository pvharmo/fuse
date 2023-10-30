use std::{ffi::OsStr};
use libc::{ENOENT, ENOTDIR, EEXIST};

use fuser::{ReplyData, ReplyEntry, Request};

use super::{FuseFS, TTL};

impl FuseFS {
    pub fn internal_readlink(&mut self, _req: &Request<'_>, ino: u64, reply: ReplyData) {
        if let Some(node) = self.tree.find_with_inode(ino) {
            if let Ok(node) = node.lock() {
                let provider = self.providers.get_provider(node.provider_id.as_ref().clone()).unwrap();
                let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    
                rt.block_on(async {
                    if let Ok(link) = provider.as_filesystem().unwrap().read_link(node.id.clone()).await {
                        if link.is_directory() {
                            reply.data(link.as_str().as_bytes())
                        } else {
                            dbg!(&link);
                            if let Ok(data) = provider.as_filesystem().unwrap().read_file(link.clone()).await {
                                reply.data(&data)
                            } else {
                                reply.error(ENOENT);
                            }
                        }
                    } else {
                        reply.error(ENOENT);
                    }
                });
            }
        }
    }

    pub fn internal_symlink(
            &mut self,
            _req: &Request<'_>,
            parent: u64,
            name: &OsStr,
            link: &std::path::Path,
            reply: ReplyEntry,
        ) {
        println!("symlink: {}", link.to_str().unwrap());

        if let Some(_) = self.tree.find_with_name(parent, name.to_str().unwrap()) {
            return reply.error(EEXIST);
        }
        
        let mut absolute_link = link;
        let mut parent_inode = parent;
        let mut link_id = None;

        if link.is_absolute() {
            absolute_link = link.strip_prefix(&self.mount_point).unwrap();
            parent_inode = 1;
        }

        let mut it = absolute_link.into_iter().peekable();

        while let Some(name) = it.next() {
            let mut node_option = self.tree.find_with_name(parent_inode, name.to_str().unwrap());
            if node_option.is_none() {
                if let Some(arc_node) = self.tree.find_with_inode(parent_inode) {
                    if let Ok(mut temp_node) = arc_node.lock() {
                        self.get_children(&mut temp_node);
                        node_option = self.tree.find_with_name(parent_inode, name.to_str().unwrap());
                        if node_option.is_none() {
                            return reply.error(ENOENT);
                        }
                    }
                }
            }
    
            if let Ok(node) = node_option.unwrap().lock() {
                link_id = Some(node.id.clone());
                if node.id.is_directory() {
                    parent_inode = node.inode;
                } else {
                    if it.peek().is_some() {
                        return reply.error(ENOTDIR);
                    }
                }
            }
        }

        if let Some(parent_node) = self.tree.find_with_inode(parent) {
            if let Ok(mut parent_node) = parent_node.lock() {
                let provider = self.providers.get_provider(parent_node.provider_id.as_ref().clone()).unwrap();
        
                let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    
                rt.block_on(async {
                    provider.as_filesystem().unwrap().create_link(parent_node.id.clone(), &name.to_str().unwrap(), link_id.unwrap()).await.unwrap();
                }); 

                self.fetch_children(&mut parent_node);
                let node = self.tree.find_with_name(parent, name.to_str().unwrap());

                return reply.entry(&TTL, &(node.unwrap().lock().unwrap().clone()).into(), 0);
            }
        }

        return reply.error(ENOENT);
    }
}