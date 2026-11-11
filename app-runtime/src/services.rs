//! Device services, as this runtime carries them: what each device's parts said they are, and carrying asks to this
//! device's parts and back (`carry`). Switching them and what oned keeps them by is services/runtime's.

use super::*;

mod carry;
pub(crate) mod routes;

impl Host {
    /// Asks each of `apps` what it is, so its service is listed as that and not as itself, then says they changed.
    pub(super) fn learn_apps(&self, apps: Vec<String>) {
        let host = self.clone();
        tokio::spawn(async move {
            for app in &apps {
                let _ = host.0.apps.describe_any(app).await;
            }
            host.emit(Event::Changed { apps });
        });
    }

    /// A service's description, as its app says it is with what the service says of its machine over it.
    pub(super) fn listed_as_app(&self, app: &str, machine: Value) -> Value {
        let mut d = self.0.apps.identity(app).unwrap_or_else(|| json!({}));
        d["name"] = json!(app);
        for (k, v) in machine.as_object().into_iter().flatten() {
            d[k] = v.clone();
        }
        d
    }

    /// Whether a part of this device is switched off, by its latest `service` entry.
    pub(super) fn off(m: &ScopeDb, device: &Key, name: &str) -> Result<bool> {
        Ok(m.switch(&m.device_of(device), name)?.is_some_and(|s| !s.on))
    }

    pub fn app_runtime(&self) -> &crate::app::runtime::Apps {
        &self.0.apps
    }

    /// Whether this device runs apps' calls: unless its user switched its app service off.
    pub fn runs_apps(&self) -> bool {
        // What it can't read, it keeps running.
        self.with_or(true, |i| Ok(!Self::off(i.r.db(), &self.0.id, scope::scopes::APPS_PART)?))
    }
}
