//! Widgets' routes: the user's widgets and badges, drawn by their apps.

pub(crate) mod routes {
    use axum::Json;
    use axum::extract::{Query, State};
    use axum::routing::get;
    use serde_json::json;

    use crate::api::{Me, Reply, Routes, Who};
    use crate::node::Node;
    use crate::says::widgets::{WidgetsAsked, WidgetsQuery};

    pub fn routes() -> Routes {
        Routes::default()
            .route("/api/widgets", Who::Clients, get(widgets))
            .route("/api/badges", Who::Clients, get(badges))
    }

    async fn widgets(State(node): State<Node>, me: Me, Query(q): Query<WidgetsAsked>) -> Reply {
        let only = q.kind.as_deref().map(|kind| kind.split_once('.').ok_or_else(|| crate::api::Invalid("a widget's kind is <app>.<widget>".into()))).transpose()?;
        Ok(Json(json!(node.widgets(me.caller, q.scope.as_deref(), only).await?)))
    }

    async fn badges(State(node): State<Node>, me: Me, Query(q): Query<WidgetsQuery>) -> Reply {
        Ok(Json(node.badges(me.caller, q.scope.as_deref()).await?))
    }
}
