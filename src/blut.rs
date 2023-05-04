
use std::{collections::HashMap, sync::{Weak, Arc, Mutex}};

use derivative::Derivative;
use nucleus_rs::{storage::ProviderId, interfaces::filesystem::ObjectId};

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
    pub provider_id: Arc<ProviderId>,
    #[derivative(PartialEq="ignore")]
    pub content_state: Arc<Mutex<FileState>>,
    #[derivative(PartialEq="ignore")]
    pub children: Arc<Mutex<Vec<Arc<FsNode>>>>,
    #[derivative(PartialEq="ignore")]
    pub parent: Weak<FsNode>,
}

#[derive(Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
pub struct File {
    pub id: ObjectId,
    pub provider_id: Arc<ProviderId>,
}

#[derive(Debug)]
pub struct ProviderAttr {
    pub id: ProviderId,
}

impl PartialEq for ProviderAttr {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

// Bidirectionnal lookup table
pub struct FsTree {
    inodes: HashMap<u64, Arc<FsNode>>,
    names: HashMap<(u64, String), Arc<FsNode>>,
    ids: HashMap<(ObjectId, ProviderId), Arc<FsNode>>,
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

        for provider_id in providers {
            blut.new_file(ObjectId::root(), provider_id.id.clone().as_str(), FileState::ShallowReady, Arc::new(provider_id), Weak::new(), Vec::new());
        }

        blut
    }

    pub fn new_file(&mut self, id: ObjectId, name: &str, content_state: FileState, provider_id: Arc<ProviderId>, parent: Weak<FsNode>, children: Vec<Arc<FsNode>>) -> Arc<FsNode> {
        let inode = self.next_inode;
        self.next_inode += 1;

        let file = Arc::new(FsNode {
            id: id.clone(),
            name: name.to_string(),
            provider_id: provider_id.clone(),
            inode,
            content_state: Arc::new(Mutex::new(content_state)),
            parent: parent.clone(),
            children: Arc::new(Mutex::new(children))
        });

        self.inodes.insert(inode, file.clone());
        self.ids.insert((id, (*provider_id).clone()), file.clone());
        if let Some(parent) = parent.upgrade() {
            self.names.insert((parent.inode, name.to_string()), file.clone());
        } else {
            self.names.insert((1, name.to_string()), file.clone());
        }

        file
    }

    pub fn find_with_inode(&self, inode: u64) -> Option<Arc<FsNode>> {
        self.inodes.get(&inode).cloned()
    }

    pub fn find_with_name(&self, parent_inode: u64, name: &str) -> Option<Arc<FsNode>> {
        self.names.get(&(parent_inode, name.to_string())).cloned()
    }

    pub fn find_with_ids(&self, object_id: ObjectId, provider_id: ProviderId) -> Option<Arc<FsNode>> {
        self.ids.get(&(object_id, provider_id)).cloned()
    }
}