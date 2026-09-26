//! Single in-flight desktop audit, worker channel lifecycle, and idle backoff.
use super::{
    audit_schedule::AuditSchedule,
    inventory::{self, Inventory},
};
use crate::pane::wake;
use desktop_core::ShellIdentity;
use desktop_shell::ShellApartment;
use std::{sync::mpsc, time::Duration};

pub(super) struct AuditResult {
    pub(super) member_keys: Vec<String>,
    pub(super) inventory: Result<Option<Inventory>, String>,
}
pub(super) struct DesktopAudit {
    requests: mpsc::Sender<(Vec<ShellIdentity>, bool)>,
    results: mpsc::Receiver<AuditResult>,
    pending: bool,
    schedule: AuditSchedule,
    wake: wake::Wake,
    restart_needed: bool,
}
impl DesktopAudit {
    pub(super) fn start(wake: wake::Wake) -> Result<Self, String> {
        let (requests, receiver) = mpsc::channel::<(Vec<ShellIdentity>, bool)>();
        let (sender, results) = mpsc::channel();
        let exit_wake = wake.clone();
        std::thread::Builder::new()
            .name("desktop-audit".into())
            .spawn(move || {
                // Declare the sender after the guard: disconnect the result
                // channel before waking the UI, including during unwinding.
                let _exit = exit_wake.on_drop();
                let sender = sender;
                let apartment = ShellApartment::initialize_sta();
                let mut reader = desktop_shell::NativeDesktopReader::default();
                let mut previous = None;
                while let Ok((managed, force)) = receiver.recv() {
                    let keys = inventory::revision_keys(&managed);
                    let result = match &apartment {
                        Ok(_) => (|| -> Result<Option<Inventory>, String> {
                            let revision = (
                                reader.revision()?,
                                desktop_shell::desktop_source_revision()
                                    .map_err(|error| error.to_string())?,
                                keys.clone(),
                            );
                            if !force && previous.as_ref() == Some(&revision) {
                                return Ok(None);
                            }
                            let snapshot = inventory::capture(&managed)?;
                            previous = Some(revision);
                            Ok(Some(snapshot))
                        })(),
                        Err(error) => Err(error.to_string()),
                    };
                    if sender
                        .send(AuditResult {
                            member_keys: keys,
                            inventory: result,
                        })
                        .is_err()
                    {
                        break;
                    }
                    exit_wake.notify();
                }
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            requests,
            results,
            pending: false,
            schedule: Default::default(),
            wake,
            restart_needed: false,
        })
    }

    pub(super) fn invalidate(&mut self) {
        self.schedule.invalidate();
    }

    pub(super) fn due(&self, elapsed: Duration) -> bool {
        !self.pending && self.schedule.due(elapsed)
    }

    pub(super) fn remaining(&self, elapsed: Duration) -> Option<Duration> {
        (!self.pending).then(|| self.schedule.remaining(elapsed))
    }

    pub(super) fn submit(
        &mut self,
        managed: Vec<ShellIdentity>,
        urgent: bool,
    ) -> Result<(), String> {
        self.submit_with(managed, urgent, Self::start)
    }

    fn submit_with(
        &mut self,
        managed: Vec<ShellIdentity>,
        urgent: bool,
        restart: impl FnOnce(wake::Wake) -> Result<Self, String>,
    ) -> Result<(), String> {
        if self.restart_needed {
            let mut fresh = restart(self.wake.clone())?;
            fresh.invalidate();
            *self = fresh;
        }
        let force = self.schedule.start();
        if self.requests.send((managed, force || urgent)).is_err() {
            return Err(self.disconnected());
        }
        self.pending = true;
        Ok(())
    }

    pub(super) fn poll(&mut self) -> Result<Option<AuditResult>, String> {
        if !self.pending {
            return Ok(None);
        }
        match self.results.try_recv() {
            Ok(result) => {
                self.pending = false;
                Ok(Some(result))
            }
            Err(mpsc::TryRecvError::Disconnected) => Err(self.disconnected()),
            Err(mpsc::TryRecvError::Empty) => Ok(None),
        }
    }

    fn disconnected(&mut self) -> String {
        self.pending = false;
        self.restart_needed = true;
        self.schedule.invalidate();
        "桌面检查线程已退出，将重建检查线程".into()
    }

    pub(super) fn complete(&mut self, changed: bool) {
        self.schedule.complete(changed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (
        DesktopAudit,
        mpsc::Receiver<(Vec<ShellIdentity>, bool)>,
        mpsc::Sender<AuditResult>,
    ) {
        let (requests, receive) = mpsc::channel();
        let (send, results) = mpsc::channel();
        (
            DesktopAudit {
                requests,
                results,
                pending: false,
                schedule: Default::default(),
                wake: Default::default(),
                restart_needed: false,
            },
            receive,
            send,
        )
    }

    #[test]
    fn disconnected_in_flight_worker_restarts_with_a_forced_capture() {
        let (mut audit, requests, results) = fixture();
        audit.submit(Vec::new(), false).unwrap();
        assert!(!requests.recv().unwrap().1);
        drop(results);
        assert!(audit.poll().is_err());
        assert!(!audit.pending);
        assert!(audit.due(Duration::ZERO));
        // A failed spawn retains the restart request for the outer retry timer.
        assert!(
            audit
                .submit_with(Vec::new(), false, |_| Err("spawn failed".into()))
                .is_err()
        );
        assert!(audit.restart_needed);
        let (fresh, requests, results) = fixture();
        audit.submit_with(Vec::new(), false, |_| Ok(fresh)).unwrap();
        assert!(requests.recv().unwrap().1);
        results
            .send(AuditResult {
                member_keys: Vec::new(),
                inventory: Ok(None),
            })
            .unwrap();
        assert!(audit.poll().unwrap().is_some());
        assert!(!audit.pending);
        assert!(!audit.restart_needed);
    }

    #[test]
    fn worker_exit_before_submission_also_allows_recreation() {
        let (mut audit, requests, _results) = fixture();
        drop(requests);
        assert!(audit.submit(Vec::new(), false).is_err());
        assert!(!audit.pending);
        assert!(audit.restart_needed);
        assert!(audit.due(Duration::ZERO));
    }
}
