//! An app drawn natively on a computer (`core/ui/src/drawing-primitives/renderer.ts`): its bundle's JS runs in a QuickJS
//! sandbox of its own, on a thread of its own (`js`), the mutations it calls build a tree (`tree`), and the screen (`view`) lays that tree out with
//! Taffy and paints it with the UI library's design tokens (`tokens`) as a scene our renderer draws (`render`, copied
//! from GPUI's), with text laid out by Parley (`text`), sending what the person does back as events.
//! An agent or a test reads and drives it with the inspector (`inspect`).

#[cfg(not(target_arch = "wasm32"))]
mod clone;
pub mod editing;
pub mod geometry;
pub mod inspect;
#[cfg(not(target_arch = "wasm32"))]
mod intl;
pub mod js;
pub mod motion;
pub mod platform;
pub mod render;
mod style;
pub mod text;
pub mod tokens;
pub mod tree;
pub(crate) mod view;
#[cfg(not(target_arch = "wasm32"))]
mod video;

pub use style::load_styles;
pub use view::inspected::Reply;
pub use view::touch;
pub use view::{Appearance, Asked, Cursor, FONT, Late, MakeFrame, Notice, Panel, Screen, Title, TitleRow, WindowButton, fonts};

/// This host's runtime version (`RUNTIME_VERSION` in core/ui/src/drawing-primitives/renderer.ts). An entrypoint that needs a
/// later one isn't drawn: a computer's window says so, and a phone opens it in its page.
pub const RUNTIME_VERSION: u32 = 5;

