// SPDX-License-Identifier: GPL-2.0-or-later
//! Bounded background asset-decoding queue for the workspace panels.
//!
//! [`Assets`] owns a worker thread and a 32-slot `SyncSender`. [`Assets::image`]
//! returns a cached image once decoding has finished; otherwise it records a
//! `None` placeholder under `category/name`, enqueues the request (removing the
//! placeholder if the queue is full) and returns `None` immediately, so panels
//! never block and only ever read completed snapshots.
//!
//! The worker resolves `Graphics/<category>/<name>` (trying the bare path then
//! `.png`/`.bmp`) into an `Arc<RgbaImage>` and stores it in the shared map that
//! `image` reads.
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{mpsc, Arc, Mutex},
};
type Images = Arc<Mutex<HashMap<String, Option<Arc<image::RgbaImage>>>>>;
pub(super) struct Assets {
    images: Images,
    requests: mpsc::SyncSender<(String, String, String)>,
}
impl Assets {
    pub fn new(root: Option<PathBuf>) -> Self {
        let images: Images = Default::default();
        let ready = images.clone();
        let (sender, receiver) = mpsc::sync_channel::<(String, String, String)>(32);
        std::thread::spawn(move || {
            let root = root.or_else(|| std::env::current_dir().ok());
            for (key, category, name) in receiver {
                let image = root
                    .as_ref()
                    .and_then(|root| {
                        let base = root.join("Graphics").join(category).join(name);
                        [
                            base.clone(),
                            base.with_extension("png"),
                            base.with_extension("bmp"),
                        ]
                        .into_iter()
                        .find_map(|p| image::open(p).ok())
                    })
                    .map(|i| Arc::new(i.to_rgba8()));
                if let Ok(mut images) = ready.lock() {
                    images.insert(key, image);
                }
            }
        });
        Self {
            images,
            requests: sender,
        }
    }
    pub fn image(&self, category: &str, name: &str) -> Option<Arc<image::RgbaImage>> {
        let key = format!("{category}/{name}");
        let mut images = self.images.lock().ok()?;
        if let Some(image) = images.get(&key) {
            return image.clone();
        }
        images.insert(key.clone(), None);
        if self
            .requests
            .try_send((key.clone(), category.into(), name.into()))
            .is_err()
        {
            images.remove(&key);
        }
        None
    }
}
