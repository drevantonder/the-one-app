//! Who asks (`api::Caller`), as this device tells its own.

use super::*;

impl Host {
    /// This device's own user, as its own programs ask.
    pub fn caller(&self) -> Result<Caller> {
        Ok(Caller::own(self.0.id, self.user()?))
    }

    pub fn this_machine(&self, caller: &Caller) -> bool {
        caller.credential == Credential::Own && caller.key == self.0.id
    }
}
