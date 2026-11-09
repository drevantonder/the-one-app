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
            .route("/v1/widgets", Who::SignedIn, get(widgets))
            .says(Op::get("The caller's widgets, drawn by their apps").query::<WidgetsQuery>())
            .route("/v1/badges", Who::SignedIn, get(badges))
            .says(Op::get("The caller's badges").query::<WidgetsQuery>())
    }

    #[derive(Deserialize, schemars::JsonSchema)]
    struct WidgetsQuery {
        scope: Option<String>,
    }

    async fn widgets(State(node): State<Node>, me: Me, Query(q): Query<WidgetsQuery>) -> Reply {
        Ok(Json(json!(node.widgets(me.caller, q.scope.as_deref()).await?)))
    }

    async fn badges(State(node): State<Node>, me: Me, Query(q): Query<WidgetsQuery>) -> Reply {
        Ok(Json(node.badges(me.caller, q.scope.as_deref()).await?))
    }
}
