//! A tool's input and answer, as every client reads them: input from flags, words, piped JSON and files, held to its
//! schema (`check`), and an answer said in words.

mod check;
mod files;
mod input;
mod words;

pub use check::{check, nearest};
pub use files::{FILE_LIMIT, as_file, mime, read_file, show};
pub use input::{Flags, from_stdin, input, kebab, missing, places, required, split, takes};
pub use words::said;
