//! Asynchronous operating-system startup operations with process-local receipts.
use super::*;
use crate::startup::Status;
struct Pending {
    id: Option<String>,
    enabled: Option<bool>,
    receiver: mpsc::Receiver<Result<(Status, Status), String>>,
}
pub(super) struct Control {
    status: Status,
    error: Option<String>,
    pending: Option<Pending>,
    checked: Option<Instant>,
}
impl Default for Control {
    fn default() -> Self {
        Self {
            status: Status::Unknown,
            error: None,
            pending: None,
            checked: None,
        }
    }
}
fn result(
    id: &str,
    operation: &str,
    commit: &str,
    status: Status,
    changed: Option<bool>,
    error: Option<String>,
) -> serde_json::Value {
    json!({"scope":"system","operation_id":id,"operation_status":operation,"commit_status":commit,
        "presentation_status":if operation=="pending"{"pending"}else if error.is_some(){"failed"}else{"applied"},
        "changed":changed,"startup_status":status.code(),"error":error})
}
impl Control {
    pub(super) fn poll(&mut self) -> Option<(String, serde_json::Value)> {
        let pending = self.pending.as_ref()?;
        let outcome = match pending.receiver.try_recv() {
            Ok(value) => value,
            Err(mpsc::TryRecvError::Empty) => return None,
            Err(mpsc::TryRecvError::Disconnected) => {
                Err("Startup worker disconnected; query OS state before retrying".into())
            }
        };
        let pending = self.pending.take().unwrap();
        self.checked = Some(Instant::now());
        let (commit, changed) = match outcome {
            Ok((before, after)) => {
                self.status = after;
                self.error = pending.enabled.and_then(|enabled| {
                    let reached = if enabled {
                        matches!(after, Status::Enabled | Status::EnabledByPolicy)
                    } else {
                        after == Status::Off
                    };
                    (!reached).then(|| {
                        format!(
                            "Windows did not reach the requested state: {}",
                            after.code()
                        )
                    })
                });
                (
                    if before == after {
                        "unchanged"
                    } else {
                        "committed"
                    },
                    Some(before != after),
                )
            }
            Err(error) => {
                self.status = Status::Unknown;
                self.error = Some(error);
                ("unknown", None)
            }
        };
        if let Some(error) = &self.error {
            crate::diagnostics::log(crate::diagnostics::Level::Error, "cli.startup", error);
        }
        pending.id.map(|id| {
            let data = result(
                &id,
                if self.error.is_some() {
                    "failed"
                } else {
                    "completed"
                },
                commit,
                self.status,
                changed,
                self.error.clone(),
            );
            (id, data)
        })
    }
    fn launch(
        &mut self,
        id: Option<String>,
        enabled: Option<bool>,
        expected: Option<Status>,
    ) -> Result<(), String> {
        if self.pending.is_some() {
            return Err("Startup operation is busy; query current request before retrying".into());
        }
        let (sender, receiver) = mpsc::channel();
        std::thread::Builder::new()
            .name("cli-startup".into())
            .spawn(move || {
                let _ = sender.send(crate::startup::operation_checked(enabled, expected));
            })
            .map_err(|e| e.to_string())?;
        self.pending = Some(Pending {
            id,
            enabled,
            receiver,
        });
        self.error = None;
        Ok(())
    }
    pub(super) fn apply(
        &mut self,
        id: &str,
        enabled: bool,
        expected_status: &str,
    ) -> serde_json::Value {
        let expected = Status::from_code(expected_status);
        match self.launch(Some(id.into()), Some(enabled), expected) {
            Ok(()) => result(id, "pending", "pending", self.status, None, None),
            Err(error) => result(
                id,
                "failed",
                "not_committed",
                self.status,
                Some(false),
                Some(error),
            ),
        }
    }
    pub(super) fn query(&mut self) -> serde_json::Value {
        if self.pending.is_none()
            && self
                .checked
                .is_none_or(|at| at.elapsed() >= Duration::from_secs(5))
        {
            if let Err(error) = self.launch(None, None, None) {
                self.error = Some(error);
                self.checked = Some(Instant::now());
            }
        }
        json!({"status":self.status.code(),"busy":self.pending.is_some(),"error":self.error,
            "editable":self.status.editable() && self.pending.is_none(),"registered":self.status.registered(),
            "effective_enabled":matches!(self.status,Status::Enabled|Status::EnabledByPolicy),
            "operation_id":self.pending.as_ref().and_then(|p|p.id.as_deref()),"cache_seconds":5})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn startup_completion_distinguishes_pending_noop_limited_and_unknown() {
        for (outcome, desired, commit, operation) in [
            (Ok((Status::Off,Status::Off)),false,"unchanged","completed"),
            (Ok((Status::Off,Status::Enabled)),true,"committed","completed"),
            (Ok((Status::Off,Status::DisabledByWindows)),true,"committed","failed"),
            (Err("injected OS failure".into()),true,"unknown","failed"),
        ] {
            let mut control=Control::default();
            let (sender,receiver)=mpsc::channel();
            control.pending=Some(Pending{id:Some("one".into()),enabled:Some(desired),receiver});
            assert!(control.poll().is_none());
            assert_eq!(control.query()["busy"],true);
            assert_eq!(control.apply("two",true,"off")["commit_status"],"not_committed");
            sender.send(outcome).unwrap();
            let (id,result)=control.poll().unwrap();
            assert_eq!(id,"one");
            assert_eq!(result["commit_status"],commit);
            assert_eq!(result["operation_status"],operation);
            assert!(control.poll().is_none());
            assert_eq!(control.query()["busy"],false);
        }
    }
}
