//! Widgets' routes: the person's widgets and badges, drawn by their apps.

pub(crate) mod routes {
    use axum::Json;
    use axum::extract::{Query, State};
    use axum::routing::get;
    use serde_json::json;

    use crate::api::{Me, Reply, Routes, Who};
    use crate::node::Node;
    use serde::Deserialize;

    pub fn routes() -> Routes {
        Routes::default()
            .route("/v1/widgets", Who::SignedIn, get(widgets))
            .route("/v1/badges", Who::SignedIn, get(badges))
    }

    #[derive(Deserialize)]
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
