
use std::{collections::HashMap, sync::{Arc, Mutex, Weak}, time::SystemTime};

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

pub struct FsTree {
    inodes: HashMap<u64, Weak<Mutex<FsNode>>>,
    names: HashMap<(u64, String), Weak<Mutex<FsNode>>>,
    ids: HashMap<(ObjectId, ProviderId), Weak<Mutex<FsNode>>>,
    next_inode: u64,
    root: Arc<Mutex<FsNode>>,
}

impl FsTree {
    pub fn new(providers: Vec<ProviderId>) -> FsTree {
        

        let root = FsNode {
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

        let mut blut = FsTree {
            inodes: HashMap::new(),
            names: HashMap::new(),
            ids: HashMap::new(),
            next_inode: 2,
            root: Arc::new(Mutex::new(root)),
        };

        for provider_id in providers {
            blut.new_provider(
                ObjectId::root(),
                provider_id.id.clone().as_str(),
                0,
                Arc::new(provider_id),
            );
        }

        blut
    }

    pub fn new_provider(&mut self, id: ObjectId, name: &str, size: u64, provider_id: Arc<ProviderId>) -> Arc<Mutex<FsNode>> {
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

        self.root.lock().unwrap().children.push(file.clone());

        self.inodes.insert(inode, Arc::downgrade(&file).clone());
        self.ids.insert((id, (*provider_id).clone()), Arc::downgrade(&file).clone());
        self.names.insert((1, name.to_string()), Arc::downgrade(&file).clone());

        file
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

        self.inodes.insert(inode, Arc::downgrade(&file).clone());
        self.ids.insert((id, (*provider_id).clone()), Arc::downgrade(&file).clone());
        self.names.insert((parent.inode, name.to_string()), Arc::downgrade(&file).clone());

        file
    }

    pub fn find_with_inode(&self, inode: u64) -> Option<Arc<Mutex<FsNode>>> {
        if let Some(node) = self.inodes.get(&inode).cloned() {
            node.upgrade()
        } else {
            None
        }
    }

    pub fn find_with_name(&self, parent_inode: u64, name: &str) -> Option<Arc<Mutex<FsNode>>> {
        if let Some(node) = self.names.get(&(parent_inode, name.to_string())).cloned() {
            node.upgrade()
        } else {
            None
        }
    }

    pub fn find_with_ids(&self, object_id: ObjectId, provider_id: ProviderId) -> Option<Arc<Mutex<FsNode>>> {
        if let Some(node) = self.ids.get(&(object_id, provider_id)).cloned() {
            node.upgrade()
        } else {
            None
        }
    }

    pub fn rename(&mut self, parent_inode: u64, old_name: &str, new_name: &str) {
        if let Some(file) = self.names.remove(&(parent_inode, old_name.to_string())) {
            self.names.insert((parent_inode, new_name.to_string()), file.clone());
        }
    }

    pub fn remove(&mut self, parent_inode: u64, node_ref: Arc<Mutex<FsNode>>) {
        let node = node_ref.lock().unwrap();

        self.inodes.remove(&node.inode);
        self.names.remove(&(parent_inode, node.name.clone()));
        self.ids.remove(&(node.id.clone(), node.provider_id.as_ref().clone()));
    }
}