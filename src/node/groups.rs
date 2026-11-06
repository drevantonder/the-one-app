//! Groups' tools: who's in the network and in each group, and handles.

use anyhow::{Result, bail};
use serde_json::{Map, Value, json};

use crate::cli::{Command, Given};
use crate::config::{CLI, YOURS};
use crate::core_tools::{Asked, Feature};

pub fn feature() -> Feature {
    let guide = format!("groups_list says who's in {YOURS}, as admin or member, then its groups: the people who share a page, its apps and its data, like a family or a team, and who's invited to one but hasn't joined. Each person and group has a handle, like andre or family, unique among them, which apps call them by (Repos' addresses are <handle>/<repo>); groups_handle changes yours, or a group's you're an admin of.");
    Feature { name: "groups", title: "Groups", description: format!("Who's in {YOURS}, and in each of its groups"), guide, tools: tools(), elsewhere: true, run }
}

fn tools() -> Vec<Value> {
    vec![
        json!({ "name": "list", "reads": true, "description": format!("The people in {YOURS}, you first, each with their handle, as admin or member; then every group with its handle: who's in it and who's invited but hasn't joined yet."), "input": { "type": "object", "properties": {} } }),
        json!({ "name": "handle", "description": "Change your handle, or a group's you're an admin of: what every app calls you or it by, like Repos' addresses <handle>/<repo>, which move with it.", "input": { "type": "object", "required": ["handle"], "properties": { "handle": { "type": "string", "description": "lowercase letters, digits and single hyphens, up to 39, not taken by anyone or any group, like andre-v" }, "group": { "type": "string", "description": "a group's name or id, from groups_list; omit for your own" } } } }),
    ]
}

fn run(tool: &str, input: &Map<String, Value>, asked: &Asked) -> Result<Value> {
    match tool {
        "list" => Ok(json!(said(&asked.node.get("/v1/state")?))),
        "handle" => {
            let handle = crate::settings_tools::HANDLE;
            let (path, body) = (handle.asks)(input).map_err(anyhow::Error::msg)?;
            Ok((handle.said)(input, &asked.node.post(path, body)?))
        }
        other => bail!("no tool groups_{other}"),
    }
}

/// A person or group as groups_list says them: their name, then their handle (`Andre @andre`).
fn handled(v: &Value) -> String {
    match v["handle"].as_str() {
        Some(h) => format!("{} @{h}", v["name"].as_str().unwrap_or("")),
        None => v["name"].as_str().unwrap_or("").to_string(),
    }
}

/// The people in the network (`/v1/state`'s), you first, each with their role; then every group in words: its people
/// with their roles, and who's invited.
pub fn said(state: &Value) -> String {
    let i = &state["network"];
    let names: std::collections::HashMap<&str, String> = i["users"].as_array().into_iter().flatten().filter_map(|u| Some((u["id"].as_str()?, handled(u)))).collect();
    let me = i["user"].as_str();
    let mut users: Vec<&Value> = i["users"].as_array().into_iter().flatten().collect();
    users.sort_by_key(|u| u["me"] != true);
    let everyone: Vec<String> = users.iter().map(|u| {
        let (name, role) = (handled(u), u["role"].as_str().unwrap_or("member"));
        if u["me"] == true { format!("{name} (you, {role})") } else { format!("{name} ({role})") }
    }).collect();
    let mut out = format!("People in {YOURS}: {}.\n", everyone.join(", "));
    let people = |list: &Value| -> Vec<String> {
        list.as_array().into_iter().flatten().map(|m| {
            let id = m["user"].as_str().unwrap_or("");
            let (name, role) = (names.get(id).map_or(id, String::as_str), m["role"].as_str().unwrap_or("member"));
            match me == Some(id) {
                true => format!("{name} (you, {role})"),
                false => format!("{name} ({role})"),
            }
        }).collect()
    };
    let groups = i["groups"].as_array().cloned().unwrap_or_default();
    if groups.is_empty() {
        out.push_str("No groups yet.\n");
        return out;
    }
    out.push_str("Groups:\n");
    for g in &groups {
        out.push_str(&format!("  {}: {}\n", handled(g), people(&g["members"]).join(", ")));
        let invited = people(&g["invited"]);
        if !invited.is_empty() {
            out.push_str(&format!("    invited, not joined yet: {}\n", invited.join(", ")));
        }
    }
    out
}

/// Making and joining groups, which aren't tools yet: Settings › Groups does them.
pub fn commands() -> Command {
    Command {
        help: vec![
            ("group create <name>".into(), "a group, with you as its admin".into()),
            ("group invite <group> <user> [--admin]".into(), "".into()),
            ("group accept <group>".into(), "".into()),
            ("group remove <group> <user>".into(), "or leave it, giving yourself".into()),
        ],
        run: cli,
    }
}

fn cli(g: &Given) -> Result<bool> {
    match g.words().as_slice() {
        ["group", "create", name @ ..] if !name.is_empty() => {
            let r = g.node.post("/v1/group", json!({ "name": name.join(" ") }))?;
            println!("Created {} ({}).", name.join(" "), r["group"].as_str().unwrap_or(""));
        }
        ["group", "invite", group, user] => {
            g.node.post("/v1/invite", json!({ "group": group, "user": user, "role": if g.has("admin") { "admin" } else { "member" } }))?;
            println!("Invited {user}; they join with {CLI} group accept {group}.");
        }
        ["group", "accept", group] => {
            g.node.post("/v1/accept", json!({ "group": group }))?;
            println!("You're in {group}.");
        }
        ["group", "remove", group, user] => {
            g.node.post("/v1/member", json!({ "group": group, "user": user, "role": null }))?;
            println!("{user} is no longer in {group}.");
        }
        _ => return Ok(false),
    }
    Ok(true)
}

pub(crate) mod routes {
    use axum::Json;
    use axum::extract::State;
    use axum::routing::post;
    use serde::Deserialize;
    use serde_json::json;

    use crate::api::{Body, Me, Reply, Routes, Who, done, given};
    use crate::database::scopes::Role;
    use crate::node::Node;

    pub fn routes() -> Routes {
        Routes::default()
            .route("/v1/group", Who::ThisMachine, post(group))
            .route("/v1/invite", Who::ThisMachine, post(invite))
            .route("/v1/accept", Who::ThisMachine, post(accept))
            .route("/v1/member", Who::ThisMachine, post(member))
            .route("/v1/handle", Who::ThisMachine, post(handle))
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Group {
        name: String,
    }

    async fn group(State(node): State<Node>, me: Me, Body(body): Body<Group>) -> Reply {
        let group = node.make_group(&me.caller, given(&body.name, "name")?).await?;
        Ok(Json(json!({ "group": group })))
    }

    /// Someone in a group, by its id or name, with their role there; `role: null` in `/v1/member` removes them.
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Member<R> {
        group: String,
        user: String,
        role: R,
    }

    async fn invite(State(node): State<Node>, me: Me, Body(body): Body<Member<Option<Role>>>) -> Reply {
        let (group, user) = (node.find_group(given(&body.group, "group")?)?, node.find_user(given(&body.user, "user")?)?);
        node.member_content(&me.caller, crate::database::content::invite(&group, &user, body.role.unwrap_or(Role::Member))).await?;
        done()
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Accept {
        group: String,
    }

    async fn accept(State(node): State<Node>, me: Me, Body(body): Body<Accept>) -> Reply {
        node.member_content(&me.caller, crate::database::content::accept(&node.find_group(given(&body.group, "group")?)?)).await?;
        done()
    }

    async fn member(State(node): State<Node>, me: Me, Body(body): Body<Member<Option<Role>>>) -> Reply {
        let (group, user) = (node.find_group(given(&body.group, "group")?)?, node.find_user(given(&body.user, "user")?)?);
        node.member_content(&me.caller, crate::database::content::member(&group, &user, body.role)).await?;
        done()
    }

    /// The caller's handle, or with `group` (its id or name) a group's they're an admin of.
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Handle {
        handle: String,
        group: Option<String>,
    }

    async fn handle(State(node): State<Node>, me: Me, Body(body): Body<Handle>) -> Reply {
        let handle = given(&body.handle, "handle")?.to_string();
        let content = match &body.group {
            Some(group) => crate::database::content::handle("group", &node.find_group(group)?, &handle),
            None => crate::database::content::handle("user", &me.caller.user, &handle),
        };
        node.member_content(&me.caller, content).await?;
        done()
    }
}
