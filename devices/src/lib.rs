//! Devices, as every build has them: letting one in, taking one out, inviting someone and where an app's schedules run,
//! as tools (`tools`), what their routes say (`says`), the command that lets someone back in with the recovery key and
//! says who holds it (`cli`), and how loaded a machine is (`load`). What serves them is the runtime's (runtime/).

pub mod says;
pub mod tools;

#[cfg(not(target_family = "wasm"))]
pub mod cli;
#[cfg(not(target_family = "wasm"))]
pub mod load;
