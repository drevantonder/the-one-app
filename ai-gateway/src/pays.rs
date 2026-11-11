//! Whose key pays for a call: as every place a call runs decides it.

use scope::scopes::Scope;
use vault::Items;

use crate::{Company, Paid};

/// The key that pays for `company`'s models in a call of `app` (none for the network's own, an agent's) by `user`, of the
/// Vault items in `scopes` (those the call reached): the most specific there is, oldest first in each.
///
/// 1. The app's own: an item of the company's key kind switched on for `app`, in any of the scopes.
/// 2. The user's: one in their own Vault switched on for no app but the core's own (`scopes::APP`).
/// 3. The network's: one in Everyone's, the same way.
///
/// An item switched on for an app is that app's alone, so it never pays another's calls; and who pays is decided
/// here, before the call, so a key that fails never falls back to the next.
pub fn pays(scopes: &[(Scope, Items)], app: Option<&str>, user: &str, company: Company) -> Option<(String, Paid)> {
    let kind = company.key_kind();
    let oldest = |scope: Option<&Scope>, mine: &dyn Fn(&vault::Item) -> bool| {
        scopes
            .iter()
            .filter(|(s, _)| scope.is_none_or(|want| s == want))
            .flat_map(|(_, items)| items.iter())
            .filter(|(_, it)| it.kind == kind && mine(it))
            .min_by_key(|(_, it)| it.made)
            .map(|(id, _)| id.clone())
    };
    let nobodys = |it: &vault::Item| it.apps.iter().all(|a| a == key::APP);
    let apps = app.and_then(|app| oldest(None, &|it| it.apps.contains(app))).map(|id| (id, Paid::App));
    apps.or_else(|| oldest(Some(&Scope::User(user.to_string())), &nobodys).map(|id| (id, Paid::User)))
        .or_else(|| oldest(Some(&Scope::Global), &nobodys).map(|id| (id, Paid::Network)))
}
