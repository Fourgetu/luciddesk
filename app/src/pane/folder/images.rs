//! Bounded CPU image cache shared by a mapping's parent/child folder sources.
use super::*;
use std::{
    collections::{BTreeMap, HashSet, VecDeque},
    sync::{Mutex, Weak},
    hash::{Hash, Hasher},
    time::{Duration, Instant},
};

pub(super) type SharedCache = Arc<Mutex<Cache>>;
const IMAGE_BYTES: usize = 64 * 1024 * 1024;
const MAX_ENTRIES: usize = 4096;
const WORKERS: usize = 3;

struct Entry {
    identity: ShellIdentity,
    image: Arc<assets::Pixels>,
    kind: String,
    modified_time: Option<std::time::SystemTime>,
    folder: bool,
    size: Option<u64>,
    used: u64,
    bytes: usize,
}

// Weak references deduplicate live pixels without keeping evicted images alive.
#[derive(Default)]
struct ImagePool {
    entries: HashMap<u64, Weak<assets::Pixels>>,
    sweep_countdown: u8,
}
impl ImagePool {
    fn intern(&mut self, image: Arc<assets::Pixels>) -> Arc<assets::Pixels> {
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        image.width.hash(&mut hash);
        image.height.hash(&mut hash);
        image.data.hash(&mut hash);
        self.intern_key(hash.finish(), image)
    }

    fn intern_key(&mut self, key: u64, image: Arc<assets::Pixels>) -> Arc<assets::Pixels> {
        if let Some(existing) = self.entries.get(&key).and_then(Weak::upgrade) {
            // Hash collisions must never substitute a different thumbnail.
            if existing.width == image.width && existing.height == image.height
                && existing.data == image.data {
                return existing;
            }
        }
        if self.entries.len() >= MAX_ENTRIES {
            // At capacity, avoid scanning thousands of live weak references on
            // every new thumbnail. Missing interning slots never block loading.
            if self.sweep_countdown == 0 {
                self.entries.retain(|_, image| image.strong_count() > 0);
                self.sweep_countdown = 255;
            } else {
                self.sweep_countdown -= 1;
            }
        }
        if self.entries.len() < MAX_ENTRIES || self.entries.contains_key(&key) {
            self.entries.insert(key, Arc::downgrade(&image));
        }
        image
    }
}

pub(super) struct Cache {
    entries: HashMap<String, Entry>,
    order: BTreeMap<u64, String>,
    clock: u64,
    bytes: usize,
    limit: usize,
    pool: ImagePool,
}

impl Default for Cache {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
            order: BTreeMap::new(),
            clock: 0,
            bytes: 0,
            limit: IMAGE_BYTES,
            pool: ImagePool::default(),
        }
    }
}

impl Cache {
    pub(super) fn touch(&mut self, keys: &[String]) {
        for key in keys {
            if let Some(entry) = self.entries.get_mut(key) {
                self.order.remove(&entry.used);
                self.clock += 1;
                entry.used = self.clock;
                self.order.insert(entry.used, key.clone());
            }
        }
    }
    fn remove(&mut self, key: &str) {
        if let Some(entry) = self.entries.remove(key) {
            self.bytes -= entry.bytes;
            self.order.remove(&entry.used);
        }
    }

    pub(super) fn restore(&mut self, item: &mut Item) -> bool {
        let key = item.identity.persistent_key();
        let Some(entry) = self.entries.get_mut(&key) else {
            return false;
        };
        // Unknown timestamps cannot prove that a file still has the same content.
        if item.details.modified_time.is_none()
            || item.details.modified_time != entry.modified_time
            || item.details.folder != entry.folder
            || item.details.size != entry.size
            || item.identity != entry.identity
        {
            self.remove(&key);
            return false;
        }
        self.order.remove(&entry.used);
        self.clock += 1;
        entry.used = self.clock;
        self.order.insert(entry.used, key);
        item.image = Some(Arc::clone(&entry.image));
        item.details.kind.clone_from(&entry.kind);
        true
    }

    pub(super) fn insert(&mut self, item: &Item) {
        let key = item.identity.persistent_key();
        self.remove(&key);
        // Failed loads are retried, so an empty image has no reusable cache value.
        let Some(image) = &item.image else { return; };
        let bytes = image.data.len();
        if bytes > self.limit {
            return;
        }
        while self.bytes + bytes > self.limit || self.entries.len() >= MAX_ENTRIES {
            let Some((_, key)) = self.order.first_key_value() else {
                break;
            };
            self.remove(&key.clone());
        }
        self.clock += 1;
        self.bytes += bytes;
        self.order.insert(self.clock, key.clone());
        self.entries.insert(
            key,
            Entry {
                identity: item.identity.clone(),
                image: Arc::clone(image),
                kind: item.details.kind.clone(),
                modified_time: item.details.modified_time,
                folder: item.details.folder,
                size: item.details.size,
                used: self.clock,
                bytes,
            },
        );
    }

    pub(super) fn retain_folder(&mut self, root: &Path, live: &HashSet<String>) {
        let removed: Vec<_> = self
            .entries
            .iter()
            .filter(|(key, entry)| {
                entry
                    .identity
                    .file_system_path()
                    .and_then(Path::parent)
                    == Some(root)
                    && !live.contains(*key)
            })
            .map(|(key, _)| key.clone())
            .collect();
        for key in removed {
            self.remove(&key);
        }
    }
}

struct Queue {
    order: VecDeque<Arc<str>>,
    pending: HashMap<Arc<str>, Item>,
}

impl Queue {
    fn new(items: Vec<Item>) -> Self {
        let mut order = VecDeque::with_capacity(items.len());
        let mut pending = HashMap::with_capacity(items.len());
        for item in items {
            let key: Arc<str> = item.identity.persistent_key().into();
            order.push_back(Arc::clone(&key));
            pending.insert(key, item);
        }
        Self { order, pending }
    }
    fn next(&mut self, priority: &[String]) -> Option<Item> {
        for key in priority {
            if let Some(item) = self.pending.remove(key.as_str()) {
                return Some(item);
            }
        }
        while let Some(key) = self.order.pop_front() {
            if let Some(item) = self.pending.remove(&key) {
                return Some(item);
            }
        }
        None
    }
}

/// Shell objects stay inside their worker's STA. Only pixel Arcs cross threads.
pub(super) fn enrich(
    items: &mut [Item],
    jobs: Vec<Item>,
    commands: &Commands,
    cache: &SharedCache,
    images: &mpsc::SyncSender<Vec<Item>>,
    wake: &wake::Wake,
) {
    run(
        jobs,
        commands,
        cache,
        |item| {
            item.details.kind = file_type(&item.identity);
            item.image = assets::load(&item.identity, 128).ok().map(Arc::new);
        },
        |batch| {
            if images.send(batch).is_ok() {
                wake.notify();
            }
        },
        items,
    );
}

fn run(
    jobs: Vec<Item>,
    commands: &Commands,
    cache: &SharedCache,
    load: impl Fn(&mut Item) + Sync,
    mut publish: impl FnMut(Vec<Item>),
    items: &mut [Item],
) {
    if jobs.is_empty() {
        return;
    }
    let queue = Mutex::new(Queue::new(jobs));
    let positions: HashMap<_, _> = items
        .iter()
        .enumerate()
        .map(|(index, item)| (item.identity.persistent_key(), index))
        .collect();
    std::thread::scope(|scope| {
        let (sender, receiver) = mpsc::sync_channel(WORKERS * 2);
        for _ in 0..WORKERS {
            let sender = sender.clone();
            let queue = &queue;
            let load = &load;
            scope.spawn(move || {
                let Ok(_apartment) = ShellApartment::initialize_sta() else {
                    return;
                };
                while !commands.stop.load(std::sync::atomic::Ordering::Acquire)
                    && commands.active.load(std::sync::atomic::Ordering::Acquire) {
                    let next = {
                        let priority = commands.priority.lock().unwrap();
                        queue.lock().unwrap().next(&priority)
                    };
                    let Some(mut item) = next else {
                        break;
                    };
                    let ready = cache.lock().unwrap().restore(&mut item);
                    if !ready {
                        load(&mut item);
                        let mut cache = cache.lock().unwrap();
                        if let Some(image) = item.image.take() {
                            item.image = Some(cache.pool.intern(image));
                        }
                        cache.insert(&item);
                    }
                    if sender.send(item).is_err() {
                        break;
                    }
                }
            });
        }
        drop(sender);
        let mut batch = Vec::new();
        let mut published = Instant::now();
        loop {
            match receiver.recv_timeout(Duration::from_millis(50)) {
                Ok(item) => {
                    if let Some(&index) = positions.get(&item.identity.persistent_key()) {
                        items[index] = item.clone();
                    }
                    batch.push(item);
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            if !batch.is_empty() && (batch.len() >= 32 || published.elapsed() >= Duration::from_millis(100)) {
                publish(std::mem::take(&mut batch));
                published = Instant::now();
            }
        }
        if !batch.is_empty() {
            publish(batch);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn item(name: &str) -> Item {
        Item {
            identity: ShellIdentity::FileSystem {
                path: PathBuf::from(format!(r"C:\test\{name}")),
                volume_id: None,
                file_id: None,
            },
            label: name.into(),
            image: None,
            details: ItemDetails {
                modified_time: Some(std::time::UNIX_EPOCH),
                ..Default::default()
            },
        }
    }
    fn load(item: &mut Item) {
        item.details.kind = "test".into();
        item.image = Some(Arc::new(assets::Pixels {
            width: 2,
            height: 1,
            data: vec![0; 8],
        }));
    }

    #[test]
    fn parallel_loading_reuses_pixels_after_navigation_and_reloads_changed_files() {
        let commands = Commands::new().unwrap();
        let cache: SharedCache = Arc::default();
        let active = AtomicUsize::new(0);
        let peak = AtomicUsize::new(0);
        let calls = AtomicUsize::new(0);
        let mut items: Vec<_> = (0..12).map(|i| item(&format!("file-{i}"))).collect();
        let mut delivered = 0;
        let start = Instant::now();
        run(
            items.clone(),
            &commands,
            &cache,
            |item| {
                let concurrent = active.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(concurrent, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(20));
                load(item);
                calls.fetch_add(1, Ordering::SeqCst);
                active.fetch_sub(1, Ordering::SeqCst);
            },
            |batch| delivered += batch.len(),
            &mut items,
        );
        eprintln!(
            "12 image jobs: {:?}, peak workers={}, patches={delivered}",
            start.elapsed(),
            peak.load(Ordering::SeqCst)
        );
        assert!((2..=WORKERS).contains(&peak.load(Ordering::SeqCst)));
        assert_eq!(delivered, 12);
        assert_eq!(calls.load(Ordering::SeqCst), 12);
        let old_images: Vec<_> = items.iter().map(|i| i.image.clone().unwrap()).collect();
        assert!(old_images.iter().all(|image| Arc::ptr_eq(image, &old_images[0])));

        // Another directory uses the same bounded cache between the two visits.
        let mut child = item(r"child\nested");
        load(&mut child);
        cache.lock().unwrap().insert(&child);
        for item in &mut items {
            item.image = None;
            item.details.kind.clear();
        }
        run(
            items.clone(),
            &commands,
            &cache,
            |_| {
                calls.fetch_add(1, Ordering::SeqCst);
            },
            |_| {},
            &mut items,
        );
        assert_eq!(
            calls.load(Ordering::SeqCst),
            12,
            "back navigation must not invoke the image loader"
        );
        assert!(
            items
                .iter()
                .zip(&old_images)
                .all(|(item, image)| Arc::ptr_eq(item.image.as_ref().unwrap(), image))
        );

        items[0].details.modified_time = Some(std::time::UNIX_EPOCH + Duration::from_secs(1));
        items[0].image = None;
        run(
            items.clone(),
            &commands,
            &cache,
            |item| {
                calls.fetch_add(1, Ordering::SeqCst);
                load(item);
                Arc::make_mut(item.image.as_mut().unwrap()).data[0] = 1;
            },
            |_| {},
            &mut items,
        );
        assert_eq!(calls.load(Ordering::SeqCst), 13);
        assert!(!Arc::ptr_eq(
            items[0].image.as_ref().unwrap(),
            &old_images[0]
        ));
    }

    #[test]
    fn slow_image_consumer_bounds_work_and_cancellation_releases_workers() {
        let commands = Commands::new().unwrap();
        let cache: SharedCache = Arc::default();
        let calls = AtomicUsize::new(0);
        let mut items: Vec<_> = (0..1000).map(|index| item(&index.to_string())).collect();
        let mut first_batch = true;
        run(items.clone(), &commands, &cache, |item| {
            load(item);
            calls.fetch_add(1, Ordering::SeqCst);
        }, |batch| {
            assert!(batch.len() <= 32);
            if first_batch {
                first_batch = false;
                std::thread::sleep(Duration::from_millis(50));
                // One delivered batch, a bounded worker queue, and one result per worker.
                assert!(calls.load(Ordering::SeqCst) <= 32 + WORKERS * 3);
                commands.stop.store(true, Ordering::Release);
            }
        }, &mut items);
        assert!(!first_batch);
        assert!(calls.load(Ordering::SeqCst) < 1000);
    }

    #[test]
    fn identical_images_share_storage_and_pool_does_not_keep_pixels_alive() {
        let mut pool = ImagePool::default();
        let images: Vec<_> = (0..1000).map(|_| pool.intern(Arc::new(assets::Pixels {
            width: 128, height: 128, data: vec![127; 128 * 128 * 4],
        }))).collect();
        assert!(images.iter().all(|image| Arc::ptr_eq(image, &images[0])));
        let unique: HashSet<_> = images.iter().map(|image| Arc::as_ptr(image)).collect();
        let bytes = unique.len() * images[0].data.len();
        eprintln!("1000 identical 128px icons: {} -> {bytes} pixel bytes", 1000 * 128 * 128 * 4);
        assert_eq!(bytes, 65536);
        let weak = Arc::downgrade(&images[0]);
        drop(images);
        assert!(weak.upgrade().is_none());

        let a = pool.intern_key(7, Arc::new(assets::Pixels { width: 1, height: 1, data: vec![0; 4] }));
        let b = pool.intern_key(7, Arc::new(assets::Pixels { width: 1, height: 1, data: vec![1; 4] }));
        assert!(!Arc::ptr_eq(&a, &b), "a hash collision must not replace pixel content");
        assert_eq!(a.data, vec![0; 4]);
        for key in 10..10000 {
            pool.intern_key(key, Arc::new(assets::Pixels { width: 1, height: 1, data: vec![0; 4] }));
        }
        assert!(pool.entries.len() <= MAX_ENTRIES);
    }

    #[test]
    fn cache_evicts_by_bytes_but_keeps_recent_visible_items_and_retries_failures() {
        let mut cache = Cache {
            limit: 16,
            ..Default::default()
        };
        for name in ["a", "b"] {
            let mut value = item(name);
            load(&mut value);
            cache.insert(&value);
        }
        cache.touch(&[item("a").identity.persistent_key()]);
        let mut value = item("c");
        load(&mut value);
        cache.insert(&value);
        assert_eq!(cache.bytes, 16);
        assert!(cache.restore(&mut item("a")));
        assert!(!cache.restore(&mut item("b")));
        assert!(cache.restore(&mut item("c")));
        let failed = item("failure");
        cache.insert(&failed);
        // A refresh must retry extraction immediately after a transient failure.
        assert!(!cache.entries.contains_key(&item("failure").identity.persistent_key()));
        assert!(!cache.restore(&mut item("failure")));
        cache.retain_folder(Path::new(r"C:\test"), &HashSet::new());
        assert_eq!(cache.bytes, 0);
        assert!(cache.entries.is_empty());
    }

    #[test]
    fn viewport_jobs_precede_background_jobs_without_duplicates() {
        let mut queue = Queue::new((0..1000).map(|i| item(&i.to_string())).collect());
        for key in &queue.order {
            let (indexed, _) = queue.pending.get_key_value(key).unwrap();
            assert!(Arc::ptr_eq(key, indexed));
        }
        let key = item("900").identity.persistent_key();
        assert_eq!(queue.next(&[key.clone()]).unwrap().label, "900");
        assert_eq!(queue.next(&[key]).unwrap().label, "0");
        let mut remaining = 0;
        while queue.next(&[]).is_some() {
            remaining += 1;
        }
        assert_eq!(remaining, 998);
    }

    #[test]
    fn navigation_cancellation_stops_queued_work() {
        let commands = Commands::new().unwrap();
        let cache: SharedCache = Arc::default();
        let calls = AtomicUsize::new(0);
        let mut items: Vec<_> = (0..1000).map(|i| item(&i.to_string())).collect();
        run(
            items.clone(),
            &commands,
            &cache,
            |item| {
                calls.fetch_add(1, Ordering::SeqCst);
                commands.stop.store(true, Ordering::Release);
                load(item);
            },
            |_| {},
            &mut items,
        );
        assert!((1..=WORKERS).contains(&calls.load(Ordering::SeqCst)));
    }
}
