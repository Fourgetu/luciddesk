//! On-demand, cancellable font discovery. Querying never saves settings.
use super::*;
#[derive(Default)]
pub(super) struct Catalog {
    key: Option<(String, String)>,
    pending: Option<fonts::CandidateLoad>,
    completed: Option<Instant>,
    names: Vec<String>,
    error: Option<String>,
    generation: u64,
}
impl Catalog {
    pub(super) fn query(&mut self) -> serde_json::Value {
        let key = (
            crate::i18n::font_sample().to_owned(),
            crate::i18n::default_font().to_owned(),
        );
        if self.key.as_ref() != Some(&key)
            || self
                .completed
                .is_some_and(|at| at.elapsed() >= Duration::from_secs(60))
        {
            self.pending = None; // Drop cancels obsolete language discovery.
            self.completed = None;
            self.names.clear();
            self.error = None;
            self.key = Some(key.clone());
            self.generation += 1;
            match fonts::CandidateLoad::start() {
                Ok(worker) => self.pending = Some(worker),
                Err(error) => {
                    self.error = Some(error);
                    self.completed = Some(Instant::now());
                }
            }
        }
        if let Some(worker) = &self.pending {
            match worker.poll() {
                Ok(names) => {
                    if names.is_empty() {
                        self.error = Some("Font discovery returned no supported families".into());
                    }
                    self.names = names;
                    self.pending = None;
                    self.completed = Some(Instant::now());
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.error = Some("Font discovery worker disconnected".into());
                    self.pending = None;
                    self.completed = Some(Instant::now());
                }
            }
        }
        json!({"busy":self.pending.is_some(),"error":self.error,"families":self.names,
            "generation":self.generation.to_string(),"default_family":key.1,"sample":key.0,
            "effective_family":fonts::family(),"cache_seconds":60})
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cached_discovery_does_not_restart_and_expiration_refreshes() {
        let mut catalog = Catalog::default();
        let deadline = Instant::now() + Duration::from_secs(30);
        let value = loop {
            let value = catalog.query();
            if value["busy"] == false {
                break value;
            }
            assert!(Instant::now() < deadline, "font discovery timed out");
            std::thread::sleep(Duration::from_millis(10));
        };
        assert!(value["error"].is_null(), "{value}");
        assert!(!value["families"].as_array().unwrap().is_empty());
        assert_eq!(catalog.query()["generation"], value["generation"]);
        catalog.completed = Some(Instant::now() - Duration::from_secs(61));
        assert_ne!(catalog.query()["generation"], value["generation"]);
    }
}
