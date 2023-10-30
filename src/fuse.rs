use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, UNIX_EPOCH, SystemTime};
use directories::{ProjectDirs, UserDirs};
use crossroads::providers::google_drive::Token;
use crossroads::providers::onedrive::token::OneDriveToken;
use std::fs;

use fuser::{FileType, FileAttr, Filesystem, ReplyAttr, ReplyEntry, Request};
use crossroads::interfaces::filesystem::{ObjectId, FileSystem};
use crossroads::providers::native_fs::NativeFs;
use crossroads::storage::{ProvidersMap, ProviderId};
use serde_json::Value;
use crossroads::storage::ProviderType;

use std::ffi::OsStr;

use crate::fstree::{FsTree, FsNode, FileState};

mod attr;
mod node;
mod dir;
mod symlink;

pub struct FuseFS {
    providers: ProvidersMap,
    tree: FsTree,
    mount_point: PathBuf,
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
    pub async fn new(mut providers: ProvidersMap, mount_point: &Path) -> Self {
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
        
        FuseFS { providers, tree: FsTree::new(providers_list), mount_point: fs::canonicalize(mount_point).unwrap().to_path_buf() }
    }

    fn get_children(&mut self, node: &mut FsNode) -> Vec<Arc<Mutex<FsNode>>> {
        if node.content_state == FileState::DeepReady {
            if let Some(expire_at) = node.expire_at {
                if expire_at > SystemTime::now() {
                    return node.children.clone();
                }
            }
        }

        return self.fetch_children(node);
    }

    fn fetch_children(&mut self, node: &mut FsNode) -> Vec<Arc<Mutex<FsNode>>> {
        let children;
        let path = node.id.clone();

        if !path.is_directory() || path.as_str().contains("fuse/mnt"){
            return Vec::new();
        }

        let fs_provider = self.providers.get_provider((*node.provider_id).clone()).unwrap();

        
        
        match node.content_state.clone() {
            FileState::ShallowReady => {
                node.content_state = FileState::Loading;

                let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
                let res = rt.block_on(async {
                    fs_provider.as_filesystem().unwrap().read_directory(path).await
                }).unwrap();

                let provider_id = node.provider_id.clone();

                for file in res {
                    println!("{}", file.name.as_str());
                    self.tree.new_file(
                        node,
                        file.id.clone(),
                        file.name.as_str(),
                        if let Some(metadata) = file.metadata { Some(metadata.into()) } else { None },
                        provider_id.clone(),
                    );
                }

                node.expire_at = Some(SystemTime::now() + Duration::from_secs(1));

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
                node.content_state = FileState::Loading;

                let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
                let res = rt.block_on(async {
                    fs_provider.as_filesystem().unwrap().read_directory(path).await
                }).unwrap();

                let provider_id = node.provider_id.clone();

                node.children.retain(|child| {
                    let child = child.lock().unwrap();
                    res.iter().find(|file| file.id == child.id).is_some()
                });

                for file in res {
                    if node.children.iter().find(|child| child.lock().unwrap().id == file.id).is_some() {
                        continue;
                    }
                    self.tree.new_file(
                        node,
                        file.id.clone(),
                        file.name.as_str(),
                        if let Some(metadata) = file.metadata { Some(metadata.into()) } else { None },
                        provider_id.clone(),
                    );
                }

                node.expire_at = Some(SystemTime::now() + Duration::from_secs(1));

                node.content_state = FileState::DeepReady;
                children = node.children.clone();
            },
        }

        children
    }
}

impl Filesystem for FuseFS {
    fn lookup(&mut self, req: &Request, parent_inode: u64, name: &OsStr, reply: ReplyEntry) {
        self.internal_lookup(req, parent_inode, name, reply)
    }

    fn getattr(&mut self, req: &Request<'_>, ino: u64, reply: ReplyAttr) {
        self.internal_getattr(req, ino, reply)
    }

    fn setattr(
            &mut self,
            req: &Request<'_>,
            ino: u64,
            mode: Option<u32>,
            uid: Option<u32>,
            gid: Option<u32>,
            size: Option<u64>,
            atime: Option<fuser::TimeOrNow>,
            mtime: Option<fuser::TimeOrNow>,
            ctime: Option<SystemTime>,
            fh: Option<u64>,
            crtime: Option<SystemTime>,
            chgtime: Option<SystemTime>,
            bkuptime: Option<SystemTime>,
            flags: Option<u32>,
            reply: ReplyAttr,
        ) {
        self.internal_setattr(req, ino, mode, uid, gid, size, atime, mtime, ctime, fh, crtime, chgtime, bkuptime, flags, reply)
    }

    fn mknod(
            &mut self,
            req: &Request<'_>,
            parent: u64,
            name: &OsStr,
            mode: u32,
            umask: u32,
            rdev: u32,
            reply: ReplyEntry,
        ) {
        self.internal_mknod(req, parent, name, mode, umask, rdev, reply)
    }

    fn unlink(&mut self, req: &Request<'_>, parent: u64, name: &OsStr, reply: fuser::ReplyEmpty) {
        self.internal_unlink(req, parent, name, reply)
    }

    fn read(
            &mut self,
            req: &Request<'_>,
            ino: u64,
            fh: u64,
            offset: i64,
            size: u32,
            flags: i32,
            lock_owner: Option<u64>,
            reply: fuser::ReplyData,
        ) {
        self.internal_read(req, ino, fh, offset, size, flags, lock_owner, reply)
    }

    fn rename(
            &mut self,
            req: &Request<'_>,
            parent: u64,
            name: &OsStr,
            newparent: u64,
            newname: &OsStr,
            flags: u32,
            reply: fuser::ReplyEmpty,
        ) {
        self.internal_rename(req, parent, name, newparent, newname, flags, reply)
    }

    fn open(&mut self, req: &Request<'_>, ino: u64, flags: i32, reply: fuser::ReplyOpen) {
        self.internal_open(req, ino, flags, reply)
    }

    fn write(
            &mut self,
            req: &Request<'_>,
            ino: u64,
            fh: u64,
            offset: i64,
            data: &[u8],
            write_flags: u32,
            flags: i32,
            lock_owner: Option<u64>,
            reply: fuser::ReplyWrite,
        ) {
        self.internal_write(req, ino, fh, offset, data, write_flags, flags, lock_owner, reply)
    }

    fn mkdir(
            &mut self,
            req: &Request<'_>,
            parent: u64,
            name: &OsStr,
            mode: u32,
            umask: u32,
            reply: ReplyEntry,
        ) {
        self.internal_mkdir(req, parent, name, mode, umask, reply)
    }

    fn rmdir(&mut self, req: &Request<'_>, parent: u64, name: &OsStr, reply: fuser::ReplyEmpty) {
        self.internal_rmdir(req, parent, name, reply)
    }

    fn readdir(
            &mut self,
            req: &Request<'_>,
            ino: u64,
            fh: u64,
            offset: i64,
            reply: fuser::ReplyDirectory,
        ) {
        self.internal_readdir(req, ino, fh, offset, reply)
    }

    fn symlink(
            &mut self,
            req: &Request<'_>,
            parent: u64,
            name: &OsStr,
            link: &Path,
            reply: ReplyEntry,
        ) {
        self.internal_symlink(req, parent, name, link, reply)
    }

    fn readlink(&mut self, req: &Request<'_>, ino: u64, reply: fuser::ReplyData) {
        self.internal_readlink(req, ino, reply)
    }
}