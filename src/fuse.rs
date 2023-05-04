use std::collections::HashMap;
use std::sync::Arc;
use std::{ffi::OsStr};
use std::os::unix::ffi::OsStrExt;
use std::time::{Duration, UNIX_EPOCH};
use directories::{ProjectDirs, UserDirs};
use libc::ENOENT;
use nucleus_rs::providers::google_drive::Token;
use nucleus_rs::providers::onedrive::token::OneDriveToken;
use std::fs;

use fuser::{FileType, FileAttr, Filesystem, ReplyAttr, ReplyData, ReplyDirectory, ReplyEntry, Request};
use nucleus_rs::interfaces::filesystem::{ObjectId, FileSystem};
use nucleus_rs::providers::native_fs::NativeFs;
use nucleus_rs::storage::{ProvidersMap, ProviderId};
use serde_json::Value;
use nucleus_rs::storage::ProviderType;

use crate::blut::{FsTree, FsNode, FileState, ProviderAttr};

pub struct FuseFS {
    providers: ProvidersMap,
    blut: FsTree,
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
    nlink: 5,
    uid: 501,
    gid: 20,
    rdev: 0,
    flags: 0,
    blksize: 512,
};

const HELLO_TXT_CONTENT: &str = "Hello World!\n";

// const HELLO_TXT_ATTR: FileAttr = FileAttr {
//     ino: 2,
//     size: 13,
//     blocks: 1,
//     atime: UNIX_EPOCH, // 1970-01-01 00:00:00
//     mtime: UNIX_EPOCH,
//     ctime: UNIX_EPOCH,
//     crtime: UNIX_EPOCH,
//     kind: FileType::RegularFile,
//     perm: 0o644,
//     nlink: 1,
//     uid: 501,
//     gid: 20,
//     rdev: 0,
//     flags: 0,
//     blksize: 512,
// };

impl FuseFS {
    pub async fn new(mut providers: ProvidersMap) -> Self {
        let storage = NativeFs { root : "".to_string() };

        if let Some(proj_dirs) = ProjectDirs::from("", "Orbital", "Files") {
            let data_dir = (proj_dirs.data_dir().to_string_lossy() + "/").to_string();
            let x = &data_dir.clone();
            if !std::path::Path::new(data_dir.as_str()).exists() {
                fs::create_dir_all(data_dir.clone()).expect(format!("Unable to create directory {}", data_dir).as_str());
            }
            let files = storage.list_folder_content(ObjectId::directory(data_dir.clone())).await.unwrap();
            println!("{}", &files.len());

    
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
                id: "My local files".to_string(),
                provider_type: nucleus_rs::storage::ProviderType::NativeFs,
            };
    
            providers.add_provider(provider, serde_json::to_value(home_path.clone()).unwrap()).await.unwrap();
        }

        let providers_list = providers.list_providers();
        
        FuseFS { providers, blut: FsTree::new(providers_list) }
    }

    fn get_provider_children(&mut self, fs_node: Arc<FsNode>, provider: &ProviderAttr) -> Vec<Arc<FsNode>> {
        let children;
        let fs_provider = self.providers.get_provider(provider.id.clone()).unwrap();
        let state = provider.content_state.lock().unwrap().to_owned().clone();

        let provider;
        let path;

        match fs_node.clone().as_ref() {
            FsNode::File(file) => {
                path = file.id;
                provider = file.provider_id.clone();
            },
            FsNode::Provider(provider) => {
                path = ObjectId::root();
            },
            _ => {}
        }

        match state {
            FileState::ShallowReady => {
                let mut state = provider.content_state.lock().unwrap();
                *state = FileState::Loading;
                drop(state);

                let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
                println!("start loading");
                let res = rt.block_on(async {
                    fs_provider.as_filesystem().unwrap().list_folder_content(path).await
                }).unwrap();

                for file in res {
                    let new_file = self.blut.new_file(
                        file.id.clone(),
                        file.name.as_str(),
                        FileState::ShallowReady,
                        provider.id.clone(),
                        Arc::downgrade(&fs_node),
                        Vec::new(),
                    );
                    provider.children.lock().unwrap().push(new_file);
                }


                let mut state = provider.content_state.lock().unwrap();
                *state = FileState::DeepReady;

                children = provider.children.lock().unwrap().clone();
            },
            FileState::Loading => {
                println!("Loading");
                while provider.content_state.lock().unwrap().to_owned() == FileState::Loading {
                    println!("sleeping");
                    std::thread::sleep(Duration::from_millis(100));
                }
                children = provider.children.lock().unwrap().clone();
            },
            FileState::DeepReady => {
                println!("DeepReady");
                children = provider.children.lock().unwrap().clone();
            },
        }

        children
    }
}

impl Filesystem for FuseFS {
    fn lookup(&mut self, _req: &Request, parent_inode: u64, name: &OsStr, reply: ReplyEntry) {
        println!("lookup: {parent_inode}, {}", name.to_str().unwrap());

        let fs_node = self.blut.find_with_name(parent_inode, name.to_str().unwrap());

        if let Some(fs_node) = fs_node {
            match fs_node.as_ref() {
                FsNode::Provider(provider) => {
                    reply.entry(&TTL, &FileAttr {
                        ino: provider.inode,
                        size: 0,
                        blocks: 0,
                        atime: UNIX_EPOCH, // 1970-01-01 00:00:00
                        mtime: UNIX_EPOCH,
                        ctime: UNIX_EPOCH,
                        crtime: UNIX_EPOCH,
                        kind: FileType::Directory,
                        perm: 0o755,
                        nlink: 0,
                        uid: 501,
                        gid: 20,
                        rdev: 0,
                        flags: 0,
                        blksize: 512,
                    }, 0);

                },
                FsNode::File(file) => {
                    reply.entry(&TTL, &FileAttr {
                        ino: file.inode,
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
                    }, 0);
                }
                _ => {}
            }
        }
    }

    fn getattr(&mut self, _req: &Request, ino: u64, reply: ReplyAttr) {
        println!("getattr: {}", ino);

        if let Some(fs_node) = self.blut.find_with_inode(ino) {
            match fs_node.as_ref() {
                FsNode::Root(_) => reply.attr(&TTL, &ROOT_DIR_ATTR),
                FsNode::Provider(_) => reply.attr(&TTL, &FileAttr {
                    ino,
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
                }),
                FsNode::File(_) => reply.attr(&TTL, &FileAttr {
                    ino,
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
                })
            };
            return;
        } else {
            reply.error(ENOENT);
        }
    }

    fn readdir(&mut self, _req: &Request, dir_inode: u64, _fh: u64, offset: i64, mut reply: ReplyDirectory) {
        println!("readdir: {}", dir_inode);

        match dir_inode {
            1 => match self.blut.find_with_inode(dir_inode) {
                Some(file_type) => {
                    if let FsNode::Root(providers) = file_type.as_ref() {
                        if offset < providers.lock().unwrap().len().try_into().unwrap() {
                            let mutex_provider = providers.lock().unwrap();
                            let arc_fs_node = mutex_provider.get(offset as usize);
                            let fs_node = arc_fs_node.unwrap().as_ref();
                            if let FsNode::Provider(provider) = fs_node {
                                let provider_name = provider.name.clone();
                                let provider_name = provider_name.as_bytes();
                                let _ = reply.add(1, offset + 1, FileType::Directory, OsStr::from_bytes(provider_name));
                            }
                        }
                    }
                },
                None => {
                    reply.error(ENOENT);
                    return;
                }
            },
            _ => {
                match offset {
                    0 => {let _ = reply.add(1, 1, FileType::Directory, OsStr::from_bytes(b"."));},
                    1 => {let _ = reply.add(1, 2, FileType::Directory, OsStr::from_bytes(b".."));},
                    _ => {
                        println!("offset: {}", offset);
                        if let Some(fs_node) = self.blut.find_with_inode(dir_inode) {
                            let children = self.get_provider_children(fs_node);
                            match fs_node.clone().as_ref() {
                                FsNode::Provider(provider) => {

                                    if offset - 2 < children.len().try_into().unwrap() {
                                        let fs_node = children.get((offset - 2) as usize).unwrap().as_ref();
                                        if let FsNode::File(file) = fs_node {
                                            let file_name = file.name.clone();
                                            let file_name = file_name.as_bytes();
                                            let _ = reply.add(file.inode, offset + 1, FileType::RegularFile, OsStr::from_bytes(file_name));
                                        }
                                    }
                                },
                                FsNode::File(file) => {
                                    let children = self.get_provider_children(fs_node, file.provider_id, file.id.clone());
                                    let file_name = file.name.clone();
                                    let file_name = file_name.as_bytes();
                                    let _ = reply.add(file.inode, offset + 1, FileType::RegularFile, OsStr::from_bytes(file_name));
                                },
                                FsNode::Root(_) => todo!(),
                            }
                        }
                    }
                }
            }
        }

        reply.ok();
    }
    
    

    fn read(&mut self, _req: &Request, ino: u64, _fh: u64, offset: i64, _size: u32, _flags: i32, _lock_owner: Option<u64>, reply: ReplyData) {
        if ino == 2 || ino == 3 {
            reply.data(&HELLO_TXT_CONTENT.as_bytes()[offset as usize..]);
        } else {
            reply.error(ENOENT);
        }
    }
}