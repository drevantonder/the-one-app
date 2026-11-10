//! Widgets' routes: the user's widgets and badges, drawn by their apps.

pub(crate) mod routes {
    use axum::Json;
    use axum::extract::{Query, State};
    use axum::routing::get;
    use serde_json::json;

    use crate::api::openapi::Op;
    use crate::api::{Me, Reply, Routes, Who};
    use crate::node::Node;
    use serde::Deserialize;

    pub fn routes() -> Routes {
        Routes::default()
            .route("/api/widgets", Who::Clients, get(widgets))
            .says(Op::get("The caller's widgets, drawn by their apps").query::<WidgetsAsked>().answers_as(json!({ "type": "array", "items": { "type": "object", "required": ["app", "widget"], "properties": { "app": { "type": "string" }, "widget": { "type": "string" }, "late": { "type": "boolean", "description": "took too long to draw beside the rest: ask for it alone, with kind" } }, "additionalProperties": true } })))
            .route("/api/badges", Who::Clients, get(badges))
            .says(Op::get("The caller's badges").query::<WidgetsQuery>().answers_as(json!({ "type": "object", "description": "each app's name → its badge, a number above 0; an app with none is left out", "additionalProperties": { "type": "number" } })))
    }

    #[derive(Deserialize, schemars::JsonSchema)]
    struct WidgetsQuery {
        scope: Option<String>,
    }

    #[derive(Deserialize, schemars::JsonSchema)]
    struct WidgetsAsked {
        scope: Option<String>,
        /// `<app>.<widget>`: that widget alone, however long it takes.
        kind: Option<String>,
    }

    async fn widgets(State(node): State<Node>, me: Me, Query(q): Query<WidgetsAsked>) -> Reply {
        let only = q.kind.as_deref().map(|kind| kind.split_once('.').ok_or_else(|| anyhow::anyhow!("a widget's kind is <app>.<widget>"))).transpose()?;
        Ok(Json(json!(node.widgets(me.caller, q.scope.as_deref(), only).await?)))
    }

    async fn badges(State(node): State<Node>, me: Me, Query(q): Query<WidgetsQuery>) -> Reply {
        Ok(Json(node.badges(me.caller, q.scope.as_deref()).await?))
    }
}
