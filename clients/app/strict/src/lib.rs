//! A strict app drawn natively on a computer (`core/ui/src/strict/protocol.ts`): its bundle's JS runs in a QuickJS
//! sandbox of its own (`js`), the ops it posts build a tree (`tree`), and the window draws that tree with GPUI's
//! elements and the UI library's design tokens (`tokens`), sending what the person does back as events (`view`).

pub mod js;
pub mod tokens;
pub mod tree;
mod view;

pub use {gpui, gpui_platform};
pub use view::{Asked, FONT, Screen, add_fonts, bind_keys, colour};

/// The version of the strict protocol this host speaks. An entrypoint that names another opens in a web view.
pub const VERSION: u32 = 1;
