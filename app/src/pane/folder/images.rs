//! Bounded CPU image cache shared by a mapping's parent/child folder sources.
use super::*;
use std::{
    collections::{BTreeMap, HashSet, VecDeque},
    sync::Mutex,
    time::{Duration, Instant},
};

pub(super) type SharedCache = Arc<Mutex<Cache>>;
const IMAGE_BYTES: usize = 64 * 1024 * 1024;
const MAX_ENTRIES: usize = 4096;
const WORKERS: usize = 3;

struct Entry {
    item: Item,
    loaded: Instant,
    used: u64,
    bytes: usize,
}

pub(super) struct Cache {
    entries: HashMap<String, Entry>,
    order: BTreeMap<u64, String>,
    clock: u64,
    bytes: usize,
    limit: usize,
}

impl Default for Cache {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
            order: BTreeMap::new(),
            clock: 0,
            bytes: 0,
            limit: IMAGE_BYTES,
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
            || item.details.modified_time != entry.item.details.modified_time
            || item.details.folder != entry.item.details.folder
            || item.details.size != entry.item.details.size
            || item.identity != entry.item.identity
            || (entry.item.image.is_none() && entry.loaded.elapsed() >= Duration::from_secs(30))
        {
            self.remove(&key);
            return false;
        }
        self.order.remove(&entry.used);
        self.clock += 1;
        entry.used = self.clock;
        self.order.insert(entry.used, key);
        item.image = entry.item.image.clone();
        item.details.kind.clone_from(&entry.item.details.kind);
        true
    }

    pub(super) fn insert(&mut self, item: Item) {
        let key = item.identity.persistent_key();
        self.remove(&key);
        let bytes = item.image.as_ref().map_or(0, |image| image.data.len());
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
                item,
                loaded: Instant::now(),
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
                    .item
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
    order: VecDeque<String>,
    pending: HashMap<String, Item>,
}

impl Queue {
    fn new(items: Vec<Item>) -> Self {
        Self {
            order: items
                .iter()
                .map(|item| item.identity.persistent_key())
                .collect(),
            pending: items
                .into_iter()
                .map(|item| (item.identity.persistent_key(), item))
                .collect(),
        }
    }
    fn next(&mut self, priority: &[String]) -> Option<Item> {
        for key in priority {
            if let Some(item) = self.pending.remove(key) {
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
    images: &mpsc::Sender<Vec<Item>>,
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
        let (sender, receiver) = mpsc::channel();
        for _ in 0..WORKERS {
            let sender = sender.clone();
            let queue = &queue;
            let load = &load;
            scope.spawn(move || {
                let Ok(_apartment) = ShellApartment::initialize_sta() else {
                    return;
                };
                while !commands.stop.load(std::sync::atomic::Ordering::Acquire) {
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
                        cache.lock().unwrap().insert(item.clone());
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
            if !batch.is_empty() && published.elapsed() >= Duration::from_millis(100) {
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

        // Another directory uses the same bounded cache between the two visits.
        let mut child = item(r"child\nested");
        load(&mut child);
        cache.lock().unwrap().insert(child);
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
    fn cache_evicts_by_bytes_but_keeps_recent_visible_items_and_retries_failures() {
        let mut cache = Cache {
            limit: 16,
            ..Default::default()
        };
        for name in ["a", "b"] {
            let mut value = item(name);
            load(&mut value);
            cache.insert(value);
        }
        cache.touch(&[item("a").identity.persistent_key()]);
        let mut value = item("c");
        load(&mut value);
        cache.insert(value);
        assert_eq!(cache.bytes, 16);
        assert!(cache.restore(&mut item("a")));
        assert!(!cache.restore(&mut item("b")));
        assert!(cache.restore(&mut item("c")));
        let failed = item("failure");
        let key = failed.identity.persistent_key();
        cache.insert(failed);
        assert!(cache.restore(&mut item("failure")));
        cache.entries.get_mut(&key).unwrap().loaded = Instant::now() - Duration::from_secs(31);
        assert!(!cache.restore(&mut item("failure")));
        cache.retain_folder(Path::new(r"C:\test"), &HashSet::new());
        assert_eq!(cache.bytes, 0);
        assert!(cache.entries.is_empty());
    }

    #[test]
    fn viewport_jobs_precede_background_jobs_without_duplicates() {
        let mut queue = Queue::new((0..1000).map(|i| item(&i.to_string())).collect());
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
