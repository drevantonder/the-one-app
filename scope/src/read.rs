//! Who's who as a device last read it, for what a feature reads of it: the scope database of `global` and each scope
//! held (`replica`), and each scope's own state (`states`).

use std::collections::{BTreeMap, HashMap};

use anyhow::Result;
use serde_json::Value;

use crate::replica::Replica;
use crate::scopes::{Scope, ScopeDb, ScopeState};
use crate::steps::ScopeReader;

pub struct Read<'a> {
    pub replica: &'a Replica,
    /// Each scope's own state, as its database says it, by its name.
    pub states: &'a HashMap<String, ScopeState>,
    /// Each scope's own database but `global`'s, of those it holds.
    pub scopes: &'a HashMap<Scope, ScopeReader>,
}

impl Read<'_> {
    pub fn db(&self) -> &ScopeDb {
        self.replica.db()
    }

    pub fn holds(&self, scope: &Scope) -> bool {
        self.replica.holds(scope)
    }

    /// `global`'s steps read, if any.
    pub fn at(&self) -> Option<u64> {
        self.replica.at()
    }

    /// A scope's own database: `global`'s is `db`.
    pub fn scope_db(&self, scope: &Scope) -> Option<&ScopeDb> {
        match scope {
            Scope::Global => Some(self.db()),
            _ => self.scopes.get(scope).map(|r| &r.db),
        }
    }

    pub fn state(&self, scope: &Scope) -> Option<&ScopeState> {
        self.states.get(&scope.to_string())
    }

    /// How `user` arranged their pages, by page, as their own scope's `arrange` entries say.
    pub fn arrangements(&self, user: &str) -> BTreeMap<String, Value> {
        self.state(&Scope::User(user.to_string())).map(|s| s.arrangements.clone()).unwrap_or_default()
    }

    /// A scope as a user calls it: Everyone, a user's name, a group's name.
    pub fn scope_name(&self, scope: &Scope) -> Result<String> {
        let db = self.db();
        Ok(match scope {
            Scope::Global => "Everyone".to_string(),
            Scope::User(u) => db.user(u)?.map(|u| u.name).unwrap_or_else(|| u.clone()),
            Scope::Group(g) => db.group(g)?.map(|g| g.name).unwrap_or_else(|| g.clone()),
        })
    }

    /// A user's name in the network, removed users' too; none if it never knew them.
    pub fn user_name(&self, user: &str) -> Result<String> {
        Ok(self.db().user_ever(user)?.map(|u| u.name).unwrap_or_default())
    }

    /// A device's name, removed ones' too; what's part of a device (a service, a native app's sign-in) by that device's.
    /// Its key, shortened, for one it never knew.
    pub fn name_of(&self, key: &key::Key) -> String {
        let of = || -> Result<Option<String>> {
            let m = self.db();
            if let Some(d) = m.machine_ever(key)?.and_then(|s| s.device)
                && let Some(machine) = m.machine_ever(&d)?
            {
                return Ok(Some(machine.name));
            }
            Ok(m.machine_ever(key)?.map(|m| m.name))
        };
        of().ok().flatten().unwrap_or_else(|| key::short(key))
    }

    /// A scope as `user` gives it: `user` (their own), `global`, `group:<id>`, or a group's name or id.
    pub fn scope_for(&self, user: &str, scope: &str) -> Result<Scope> {
        Scope::parse_for(scope, user).or_else(|_| self.db().find_group(scope).map(Scope::Group))
    }
}
