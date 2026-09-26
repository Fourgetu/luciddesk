//! Keep a small, short-lived reuse window for icons moved back to Explorer.
use super::{Arc, HashMap, assets};
use std::{
    collections::HashSet,
    time::{Duration, Instant},
};

const MAX_BYTES: usize = 8 * 1024 * 1024;
const MAX_ITEMS: usize = 64;
pub(super) const TTL: Duration = Duration::from_secs(30);

#[derive(Default)]
pub(super) struct ImageRetention {
    released: HashMap<String, Instant>,
}

impl ImageRetention {
    pub(super) fn deadline(&self) -> Option<Instant> {
        self.released.values().min().map(|time| *time + TTL)
    }

    pub(super) fn retain(
        &mut self,
        live: &HashSet<String>,
        images: &mut HashMap<String, Arc<assets::Pixels>>,
        now: Instant,
    ) {
        self.released
            .retain(|key, _| !live.contains(key) && images.contains_key(key));
        for key in images.keys().filter(|key| !live.contains(*key)) {
            self.released.entry(key.clone()).or_insert(now);
        }
        self.expire(images, now);
        let mut bytes: usize = self
            .released
            .keys()
            .filter_map(|key| images.get(key))
            .map(|image| image.data.capacity())
            .sum();
        if bytes <= MAX_BYTES && self.released.len() <= MAX_ITEMS {
            return;
        }
        let mut oldest: Vec<_> = self
            .released
            .iter()
            .map(|(key, time)| (key.clone(), *time))
            .collect();
        oldest.sort_unstable_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
        for (key, _) in oldest {
            if bytes <= MAX_BYTES && self.released.len() <= MAX_ITEMS {
                break;
            }
            self.released.remove(&key);
            if let Some(image) = images.remove(&key) {
                bytes -= image.data.capacity();
            }
        }
    }

    pub(super) fn expire(
        &mut self,
        images: &mut HashMap<String, Arc<assets::Pixels>>,
        now: Instant,
    ) {
        self.released.retain(|key, released| {
            if now.saturating_duration_since(*released) >= TTL {
                images.remove(key);
                false
            } else {
                images.contains_key(key)
            }
        });
    }

    // Shell notifications and inventory changes invalidate inactive pixels;
    // active icons continue through the existing targeted refresh path.
    pub(super) fn invalidate(&mut self, images: &mut HashMap<String, Arc<assets::Pixels>>) {
        for (key, _) in self.released.drain() {
            images.remove(&key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(bytes: usize) -> Arc<assets::Pixels> {
        Arc::new(assets::Pixels {
            width: 128,
            height: 128,
            data: vec![0; bytes],
        })
    }

    #[test]
    fn quick_return_reuses_pixels_and_does_not_expire_active_icons() {
        let now = Instant::now();
        let mut cache = ImageRetention::default();
        let pixels = image(65536);
        let mut images = HashMap::from([("a".into(), pixels.clone())]);
        cache.retain(&HashSet::new(), &mut images, now);
        assert_eq!(cache.deadline(), Some(now + TTL));
        cache.retain(
            &HashSet::from(["a".into()]),
            &mut images,
            now + Duration::from_secs(1),
        );
        cache.expire(&mut images, now + TTL);
        assert_eq!(cache.deadline(), None);
        assert!(Arc::ptr_eq(&pixels, &images["a"]));
        cache.retain(&HashSet::new(), &mut images, now + TTL);
        cache.expire(&mut images, now + TTL * 2);
        assert!(images.is_empty());
    }

    #[test]
    fn budget_evicts_oldest_inactive_images_and_preserves_active_images() {
        let now = Instant::now();
        let mut cache = ImageRetention::default();
        let mut images = HashMap::from([("old".into(), image(MAX_BYTES))]);
        cache.retain(&HashSet::new(), &mut images, now);
        images.insert("recent".into(), image(65536));
        images.insert("active".into(), image(MAX_BYTES + 1));
        cache.retain(
            &HashSet::from(["active".into()]),
            &mut images,
            now + Duration::from_secs(1),
        );
        assert!(!images.contains_key("old"));
        assert!(images.contains_key("recent"));
        assert!(images.contains_key("active"));
        cache.invalidate(&mut images);
        assert_eq!(images.len(), 1);
        assert!(images.contains_key("active"));
    }

    #[test]
    fn item_limit_and_original_expiry_bound_idle_retention() {
        let now = Instant::now();
        let mut cache = ImageRetention::default();
        let mut images = (0..100).map(|i| (i.to_string(), image(4))).collect();
        cache.retain(&HashSet::new(), &mut images, now);
        assert_eq!(images.len(), MAX_ITEMS);
        cache.retain(
            &HashSet::new(),
            &mut images,
            now + TTL - Duration::from_secs(1),
        );
        cache.expire(&mut images, now + TTL);
        assert!(images.is_empty(), "maintenance must not extend the expiry");
    }
}
