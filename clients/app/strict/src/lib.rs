//! A strict app drawn natively on a computer (`core/ui/src/strict/renderer.ts`): its bundle's JS runs in a QuickJS
//! sandbox of its own, on a thread of its own (`js`), the mutations it calls build a tree (`tree`), and the screen (`view`) lays that tree out with
//! Taffy and paints it with the UI library's design tokens (`tokens`) as a scene our renderer draws (`render`, copied
//! from GPUI's), with text from the platform's text system (`text`), sending what the person does back as events.
//! An agent or a test reads and drives it with the inspector (`inspect`).

mod clone;
pub mod editing;
pub mod geometry;
pub mod inspect;
mod intl;
pub mod js;
pub mod platform;
pub mod render;
mod style;
pub mod text;
pub mod tokens;
pub mod tree;
pub(crate) mod view;

pub use style::{colour, load_styles};
pub use view::inspected::Reply;
pub use view::{Appearance, Asked, Breadcrumb, Cursor, FONT, Notice, Panel, Screen, fonts};

/// This host's runtime version (`RUNTIME_VERSION` in core/ui/src/strict/renderer.ts). An entrypoint that needs a
/// later one opens in a web view.
pub const RUNTIME_VERSION: u32 = 5;

