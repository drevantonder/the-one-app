//! The apps this build ships with. Only the binary and tests name them; the core runs whatever it's given.

use crate::native::{Builtin, Native};

pub fn all() -> Builtin {
    Builtin {
        apps: vec![("board".into(), include_str!("../apps/board.js").into())],
        native: vec![
            Native { name: "shell", describe: crate::shell::describe, entries: false, call: crate::shell::call, sync: None },
            Native { name: "agents", describe: crate::agents::describe, entries: true, call: crate::agents::call, sync: Some(crate::agents::sync) },
            Native { name: "models", describe: crate::models::describe, entries: false, call: crate::models::call, sync: None },
        ],
    }
}
