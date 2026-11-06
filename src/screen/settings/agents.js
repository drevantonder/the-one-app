// Agents, as an item's apps and a request name them: the node's `scopes::AGENTS`, which tests/contracts.rs holds this
// to. They're no app, so /v1/apps doesn't say who they are.
export const AGENTS = { name: "ai-agents", title: "Agents", icon: "robot", color: "#8e8e93" }

// An app of /v1/apps by its name, or Agents.
export const appNamed = (apps, name) => (name === AGENTS.name ? AGENTS : apps.find((a) => a.name === name))
