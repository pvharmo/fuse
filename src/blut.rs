
use std::{collections::HashMap, sync::{Arc, Mutex}, time::SystemTime};

use derivative::Derivative;
use crossroads::{storage::ProviderId, interfaces::filesystem::ObjectId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FileState {
    ShallowReady,
    Loading,
    DeepReady,
}

#[derive(Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
pub struct FsNode {
    pub id: ObjectId,
    pub inode: u64,
    pub name: String,
    pub size: u64,
    pub blocks: u64,
    pub atime: SystemTime,
    pub mtime: SystemTime,
    pub ctime: SystemTime,
    pub crtime: SystemTime,
    pub perm: u16,
    pub uid: u32,
    pub gid: u32,
    pub rdev: u32,
    pub blksize: u32,
    pub flags: u32,
    pub provider_id: Arc<ProviderId>,
    #[derivative(PartialEq="ignore")]
    pub content_state: FileState,
    #[derivative(PartialEq="ignore")]
    pub children: Vec<Arc<Mutex<FsNode>>>,
}

// Bidirectionnal lookup table
pub struct FsTree {
    inodes: HashMap<u64, Arc<Mutex<FsNode>>>,
    names: HashMap<(u64, String), Arc<Mutex<FsNode>>>,
    ids: HashMap<(ObjectId, ProviderId), Arc<Mutex<FsNode>>>,
    next_inode: u64,
}

impl FsTree {
    pub fn new(providers: Vec<ProviderId>) -> FsTree {
        let mut blut = FsTree {
            inodes: HashMap::new(),
            names: HashMap::new(),
            ids: HashMap::new(),
            next_inode: 2,
        };

        let mut root = FsNode {
            id: ObjectId::root(),
            name: "/".to_string(),
            provider_id: Arc::new(ProviderId {id: "".to_string(), provider_type: crossroads::storage::ProviderType::NativeFs}),
            inode: 1,
            size: 0,
            blocks: 0,
            atime: SystemTime::UNIX_EPOCH,
            mtime: SystemTime::UNIX_EPOCH,
            ctime: SystemTime::UNIX_EPOCH,
            crtime: SystemTime::UNIX_EPOCH,
            perm: 0o777,
            uid: 501,
            gid: 20,
            rdev: 0,
            blksize: 0,
            flags: 0,
            content_state: FileState::ShallowReady,
            children: Vec::new()
        };

        for provider_id in providers {
            blut.new_file(
                &mut root,
                ObjectId::root(),
                provider_id.id.clone().as_str(),
                0,
                Arc::new(provider_id),
            );
        }

        blut
    }

    pub fn new_file(&mut self, parent: &mut FsNode, id: ObjectId, name: &str, size: u64, provider_id: Arc<ProviderId>) -> Arc<Mutex<FsNode>> {
        let inode = self.next_inode;
        self.next_inode += 1;

        let file = Arc::new(Mutex::new(FsNode {
            id: id.clone(),
            name: name.to_string(),
            provider_id: provider_id.clone(),
            inode,
            size,
            blocks: 0,
            atime: SystemTime::UNIX_EPOCH,
            mtime: SystemTime::UNIX_EPOCH,
            ctime: SystemTime::UNIX_EPOCH,
            crtime: SystemTime::UNIX_EPOCH,
            perm: 0o777,
            uid: 501,
            gid: 20,
            rdev: 0,
            blksize: 0,
            flags: 0,
            content_state: FileState::ShallowReady,
            children: Vec::new()
        }));

        parent.children.push(file.clone());

        self.inodes.insert(inode, file.clone());
        self.ids.insert((id, (*provider_id).clone()), file.clone());
        self.names.insert((parent.inode, name.to_string()), file.clone());

        file
    }

    pub fn find_with_inode(&self, inode: u64) -> Option<Arc<Mutex<FsNode>>> {
        self.inodes.get(&inode).cloned()
    }

    pub fn find_with_name(&self, parent_inode: u64, name: &str) -> Option<Arc<Mutex<FsNode>>> {
        self.names.get(&(parent_inode, name.to_string())).cloned()
    }

    pub fn find_with_ids(&self, object_id: ObjectId, provider_id: ProviderId) -> Option<Arc<Mutex<FsNode>>> {
        self.ids.get(&(object_id, provider_id)).cloned()
    }
}