use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::{ffi::OsStr};
use std::os::unix::ffi::OsStrExt;
use std::time::{Duration, UNIX_EPOCH, SystemTime};
use directories::{ProjectDirs, UserDirs};
use libc::ENOENT;
use crossroads::providers::google_drive::Token;
use crossroads::providers::onedrive::token::OneDriveToken;
use std::fs;
use chrono;

use fuser::{FileType, FileAttr, Filesystem, ReplyAttr, ReplyData, ReplyDirectory, ReplyEntry, Request};
use crossroads::interfaces::filesystem::{ObjectId, FileSystem, File, Metadata};
use crossroads::providers::native_fs::NativeFs;
use crossroads::storage::{ProvidersMap, ProviderId};
use serde_json::Value;
use crossroads::storage::ProviderType;

use crate::fstree::{FsTree, FsNode, FileState};

pub struct FuseFS {
    providers: ProvidersMap,
    tree: FsTree,
}

const TTL: Duration = Duration::from_secs(1);

const ROOT_DIR_ATTR: FileAttr = FileAttr {
    ino: 1,
    size: 0,
    blocks: 0,
    atime: UNIX_EPOCH, // 1970-01-01 00:00:00
    mtime: UNIX_EPOCH,
    ctime: UNIX_EPOCH,
    crtime: UNIX_EPOCH,
    kind: FileType::Directory,
    perm: 0o755,
    nlink: 4,
    uid: 501,
    gid: 20,
    rdev: 0,
    flags: 0,
    blksize: 512,
};

impl FuseFS {
    pub async fn new(mut providers: ProvidersMap) -> Self {
        let storage = NativeFs { root : "".to_string() };

        if let Some(proj_dirs) = ProjectDirs::from("", "Orbital", "Files") {
            let data_dir = (proj_dirs.data_dir().to_string_lossy() + "/").to_string();
            let x = &data_dir.clone();
            if !std::path::Path::new(data_dir.as_str()).exists() {
                fs::create_dir_all(data_dir.clone()).expect(format!("Unable to create directory {}", data_dir).as_str());
            }
            let files = storage.read_directory(ObjectId::directory(data_dir.clone())).await.unwrap();
    
            for file in files {
                let path = x.clone() + "/" + file.name.as_str();
                let content = storage.read_file(ObjectId::plain_text(path.clone())).await.unwrap();
                let file_name_split: Vec<&str> = file.name.splitn(2, ".").collect();
    
    
                let content_string = String::from_utf8(content).unwrap();
    
                match file_name_split[1] {
                    "S3" => {
                        let credentials: Value = serde_json::from_str(content_string.as_str()).unwrap();
                        providers.add_provider(ProviderId { id: file_name_split[0].to_string(), provider_type: ProviderType::S3 }, credentials).await.unwrap();
                    },
                    "GoogleDrive" => {
                        let tokens: HashMap<String, Token> = serde_json::from_str(content_string.as_str()).unwrap();
                        providers.add_provider(ProviderId { id: file_name_split[0].to_string(), provider_type: ProviderType::GoogleDrive }, serde_json::to_value(tokens).unwrap()).await.unwrap();
                    },
                    "OneDrive" => {
                        let token: Option<OneDriveToken> = serde_json::from_str(content_string.as_str()).unwrap();
                        providers.add_provider(ProviderId { id: file_name_split[0].to_string(), provider_type: ProviderType::OneDrive }, serde_json::to_value(token).unwrap()).await.unwrap();
                    },
                    _ => ()
                }
                
            }
        }
    
        if let Some(user_dirs) = UserDirs::new() {
            let home_path = (user_dirs.home_dir().to_string_lossy() + "/").to_string();
            let provider = ProviderId {
                id: "Local files".to_string(),
                provider_type: crossroads::storage::ProviderType::NativeFs,
            };
    
            providers.add_provider(provider, serde_json::to_value(home_path.clone()).unwrap()).await.unwrap();
        }

        let providers_list = providers.list_providers();
        
        FuseFS { providers, tree: FsTree::new(providers_list) }
    }

    fn get_children(&mut self, node: &mut FsNode) -> Vec<Arc<Mutex<FsNode>>> {
        let children;
        let path = node.id.clone();

        if !path.is_directory() || path.as_str().contains("fuse/mnt"){
            return Vec::new();
        }

        let fs_provider = self.providers.get_provider((*node.provider_id).clone()).unwrap();
        
        match node.content_state.clone() {
            FileState::ShallowReady => {
                node.content_state = FileState::Loading;

                dbg!(&path);

                let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
                let res = rt.block_on(async {
                    fs_provider.as_filesystem().unwrap().read_directory(path).await
                }).unwrap();

                let provider_id = node.provider_id.clone();

                for file in res {
                    self.tree.new_file(
                        node,
                        file.id.clone(),
                        file.name.as_str(),
                        if let Some(metadata) = file.metadata { Some(metadata.into()) } else { None },
                        provider_id.clone(),
                    );
                }

                node.content_state = FileState::DeepReady;

                children = node.children.clone();
            },
            FileState::Loading => {
                while node.content_state == FileState::Loading {
                    std::thread::sleep(Duration::from_millis(100));
                }
                children = node.children.clone();
            },
            FileState::DeepReady => {
                children = node.children.clone();
            },
        }

        children
    }
}

impl Filesystem for FuseFS {
    fn lookup(&mut self, _req: &Request, parent_inode: u64, name: &OsStr, reply: ReplyEntry) {
        println!("lookup: {parent_inode}, {}", name.to_str().unwrap());
        
        if let Some(fs_node) = self.tree.find_with_name(parent_inode, name.to_str().unwrap()) {
            if let Ok(node) = fs_node.lock() {
                reply.entry(&TTL, &(*node).clone().into(), 0);
            }
        } else {
            reply.error(ENOENT);
        }
    }

    fn setattr(
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

    fn getattr(&mut self, _req: &Request, ino: u64, reply: ReplyAttr) {
        println!("getattr: {}", ino);

        if ino == 1 {
            reply.attr(&TTL, &ROOT_DIR_ATTR);
            return;
        }

        if let Some(fs_node) = self.tree.find_with_inode(ino) {
            if let Ok(node) = fs_node.lock() {
                reply.attr(&TTL, &(*node).clone().into());
            }
        } else {
            reply.error(ENOENT);
        }
    }

    fn readdir(&mut self, _req: &Request, dir_inode: u64, _fh: u64, offset: i64, mut reply: ReplyDirectory) {
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

    fn unlink(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: fuser::ReplyEmpty) {
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

    fn rmdir(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: fuser::ReplyEmpty) {
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

    fn mkdir(
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
                        metadata: Some(Metadata {
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

                    dbg!(_mode);
                    dbg!(_umask);

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

    fn mknod(
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
                    let id = ObjectId::new(parent_dir.id.to_string() + "/" + name.to_str().unwrap(), None);
                    provider.as_filesystem().unwrap().create(parent_dir.id.clone(), File {
                        id: id.clone(),
                        name: name.to_str().unwrap().to_string(),
                        metadata: Some(Metadata {
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
    
    fn read(&mut self, _req: &Request, ino: u64, _fh: u64, offset: i64, size: u32, _flags: i32, _lock_owner: Option<u64>, reply: ReplyData) {
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

    fn rename(
            &mut self,
            _req: &Request<'_>,
            parent: u64,
            name: &OsStr,
            newparent: u64,
            newname: &OsStr,
            _flags: u32,
            reply: fuser::ReplyEmpty,
        ) {
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

    fn open(&mut self, _req: &Request<'_>, _ino: u64, _flags: i32, reply: fuser::ReplyOpen) {
        reply.opened(0, 0)
    }

    fn write(
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