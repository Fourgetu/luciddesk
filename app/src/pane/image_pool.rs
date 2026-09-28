//! Share identical CPU pixels across desktop groups and folder sources.
use super::assets;
use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    sync::{Arc, Mutex, OnceLock, Weak},
};

const MAX_ENTRIES: usize = 4096;

pub(super) fn intern(image: Arc<assets::Pixels>) -> Arc<assets::Pixels> {
    static POOL: OnceLock<Mutex<ImagePool>> = OnceLock::new();
    // Hash outside the lock so independent Shell workers can run concurrently.
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    image.width.hash(&mut hash);
    image.height.hash(&mut hash);
    image.data.hash(&mut hash);
    POOL.get_or_init(Mutex::default)
        .lock()
        .unwrap()
        .intern_key(hash.finish(), image)
}

// Weak references deduplicate live pixels without keeping evicted images alive.
#[derive(Default)]
struct ImagePool {
    entries: HashMap<u64, Weak<assets::Pixels>>,
    sweep_countdown: u8,
}
impl ImagePool {
    fn intern_key(&mut self, key: u64, image: Arc<assets::Pixels>) -> Arc<assets::Pixels> {
        if let Some(existing) = self.entries.get(&key).and_then(Weak::upgrade) {
            // Hash collisions must never substitute a different thumbnail.
            if existing.width == image.width
                && existing.height == image.height
                && existing.data == image.data
            {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn independent_threads_share_pixels_without_extending_their_lifetime() {
        let make = || {
            Arc::new(assets::Pixels {
                width: 128,
                height: 128,
                data: vec![93; 128 * 128 * 4],
            })
        };
        let images: Vec<_> = std::thread::scope(|scope| {
            let workers: Vec<_> = (0..16).map(|_| scope.spawn(|| intern(make()))).collect();
            workers
                .into_iter()
                .map(|worker| worker.join().unwrap())
                .collect()
        });
        assert!(images.iter().all(|image| Arc::ptr_eq(image, &images[0])));
        let weak = Arc::downgrade(&images[0]);
        drop(images);
        assert!(weak.upgrade().is_none());

        let original = intern(make());
        let mut changed = make();
        Arc::make_mut(&mut changed).data[0] = 94;
        let changed = intern(changed);
        assert!(!Arc::ptr_eq(&original, &changed));
        assert_eq!(original.data[0], 93);
        assert_eq!(changed.data[0], 94);
    }

    #[test]
    fn identical_images_share_storage_and_pool_does_not_keep_pixels_alive() {
        let mut pool = ImagePool::default();
        let images: Vec<_> = (0..1000)
            .map(|_| {
                pool.intern_key(
                    1,
                    Arc::new(assets::Pixels {
                        width: 128,
                        height: 128,
                        data: vec![127; 128 * 128 * 4],
                    }),
                )
            })
            .collect();
        assert!(images.iter().all(|image| Arc::ptr_eq(image, &images[0])));
        let unique: HashSet<_> = images.iter().map(|image| Arc::as_ptr(image)).collect();
        let bytes = unique.len() * images[0].data.len();
        eprintln!(
            "1000 identical 128px icons: {} -> {bytes} pixel bytes",
            1000 * 128 * 128 * 4
        );
        assert_eq!(bytes, 65536);
        let weak = Arc::downgrade(&images[0]);
        drop(images);
        assert!(weak.upgrade().is_none());

        let a = pool.intern_key(
            7,
            Arc::new(assets::Pixels {
                width: 1,
                height: 1,
                data: vec![0; 4],
            }),
        );
        let b = pool.intern_key(
            7,
            Arc::new(assets::Pixels {
                width: 1,
                height: 1,
                data: vec![1; 4],
            }),
        );
        assert!(
            !Arc::ptr_eq(&a, &b),
            "a hash collision must not replace pixel content"
        );
        assert_eq!(a.data, vec![0; 4]);
        for key in 10..10000 {
            pool.intern_key(
                key,
                Arc::new(assets::Pixels {
                    width: 1,
                    height: 1,
                    data: vec![0; 4],
                }),
            );
        }
        assert!(pool.entries.len() <= MAX_ENTRIES);
    }
}
