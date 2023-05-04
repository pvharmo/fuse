use nucleus_rs::storage::*;

mod fuse;
mod mount;
mod blut;

fn main() {
    let options = ProvidersOptions {
        google_api_key: Some(env!("GOOGLE_DRIVE_CLIENT_KEY").to_string()),
        onedrive_api_key: Some(env!("ONEDRIVE_CLIENT_ID").to_string())
    };

    let mut fs = None;

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let providers: ProvidersMap = ProvidersMap::new(options).await;
        
            // Instantiate the filesystem
            fs = Some(fuse::FuseFS::new(providers).await);
        });

    // Instantiate the mountpoint
    let mountpoint = mount::Mount::new("./mnt");

    // Mount the filesystem
    mountpoint.mount(fs.unwrap()).unwrap();
}