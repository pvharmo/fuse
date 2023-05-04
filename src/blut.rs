
use std::{collections::HashMap, sync::{Weak, Arc, Mutex}};

use derivative::Derivative;
use nucleus_rs::{storage::ProviderId, interfaces::filesystem::ObjectId};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Id {
    Root,
    Provider(ProviderId),
    File(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FileState {
    ShallowReady,
    Loading,
    DeepReady,
}

#[derive(Debug)]
pub enum FsNode {
    Root(Mutex<Vec<Arc<FsNode>>>),
    Provider(ProviderAttr),
    File(File),
}

impl PartialEq for FsNode {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Root(_), Self::Root(_)) => true,
            (Self::Provider(l0), Self::Provider(r0)) => l0 == r0,
            (Self::File(l0), Self::File(r0)) => l0 == r0,
            _ => false,
        }
    }
}

#[derive(Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
pub struct File {
    pub id: ObjectId,
    pub name: String,
    pub inode: u64,
    pub content_state: FileState,
    pub provider_id: Arc<ProviderId>,
    #[derivative(PartialEq="ignore")]
    #[derivative(Hash="ignore")]
    pub parent: Weak<FsNode>,
    pub children: Vec<Arc<FsNode>>,
}

#[derive(Debug)]
pub struct ProviderAttr {
    pub id: ProviderId,
    pub name: String,
    pub inode: u64,
    pub content_state: Mutex<FileState>,
    pub children: Mutex<Vec<Arc<FsNode>>>,
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
    ids: HashMap<Id, Arc<FsNode>>,
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

        let root = Arc::new(FsNode::Root(Mutex::new(Vec::new())));

        blut.inodes.insert(1, root.clone());
        blut.names.insert((1, "/".to_string()), root.clone());
        blut.ids.insert(Id::Root, root);

        for provider_id in providers {
            blut.new_provider(&provider_id, provider_id.id.clone().as_str(), FileState::ShallowReady, Vec::new());
        }

        blut
    }

    pub fn new_file(&mut self, id: ObjectId, name: &str, content_state: FileState, provider_id: Arc<ProviderId>, parent: Weak<FsNode>, children: Vec<Arc<FsNode>>) -> Arc<FsNode> {
        let inode = self.next_inode;
        self.next_inode += 1;

        let file = Arc::new(FsNode::File(File {
            id,
            name: name.to_string(),
            provider_id,
            inode,
            content_state,
            parent,
            children
        }));

        self.inodes.insert(inode, file.clone());
        self.names.insert((inode, name.to_string()), file.clone());
        self.ids.insert(Id::File(id.to_string()), file.clone());

        file
    }

    pub fn new_provider(&mut self, id: &ProviderId, name: &str, content_state: FileState, children: Vec<Arc<FsNode>>) -> Arc<FsNode> {
        let inode = self.next_inode;
        self.next_inode += 1;

        let provider = Arc::new(FsNode::Provider(ProviderAttr { id: id.clone(), name: name.to_string(), inode, content_state: Mutex::new(content_state), children: Mutex::new(children)}));

        self.inodes.insert(inode, provider.clone());
        self.names.insert((1, name.to_string()), provider.clone());
        self.ids.insert(Id::Provider(id.clone()), provider.clone());

        match self.find_with_inode(1).unwrap().as_ref() {
            FsNode::Root(providers) => providers.to_owned().lock().unwrap().push(provider.clone()),
            _ => panic!("Root is not a root")
        }

        provider
    }

    pub fn find_with_inode(&self, inode: u64) -> Option<Arc<FsNode>> {
        self.inodes.get(&inode).cloned()
    }

    pub fn find_with_name(&self, parent_inode: u64, name: &str) -> Option<Arc<FsNode>> {
        self.names.get(&(parent_inode, name.to_string())).cloned()
    }

    pub fn find_with_ids(&self, id: &Id) -> Option<Arc<FsNode>> {
        self.ids.get(id).cloned()
    }
}