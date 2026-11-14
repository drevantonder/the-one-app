//! SQLite for plugins on a server. A server gives it to plugins as something to import (`Sqlite`), and One Core
//! doesn't use it: what a database is, beyond a file SQLite opens, is the database plugin's.

use std::sync::Mutex;

use rusqlite::fallible_iterator::FallibleIterator;
use rusqlite::types::{Value, ValueRef};
use rusqlite::{Batch, Connection};
use rustler::types::atom::{false_, nil, true_};
use rustler::{Atom, Binary, Encoder, Env, NewBinary, Resource, ResourceArc, Term, TermType};

mod atoms {
    rustler::atoms! { ok, blob }
}

/// An open database file. Closed when Elixir lets go of it, or when it's told to.
struct Database(Mutex<Option<Connection>>);

#[rustler::resource_impl]
impl Resource for Database {}

/// Opens the database file at `path`, making it if there's none.
#[rustler::nif(schedule = "DirtyIo")]
fn open(path: String) -> Result<ResourceArc<Database>, String> {
    let connection = Connection::open(path).map_err(|error| error.to_string())?;
    Ok(ResourceArc::new(Database(Mutex::new(Some(connection)))))
}

/// Runs one statement with its values, and gives its rows: each a map from a column's name to its value.
#[rustler::nif(schedule = "DirtyIo")]
fn query<'a>(
    env: Env<'a>,
    database: ResourceArc<Database>,
    sql: &str,
    values: Vec<Term<'a>>,
) -> Result<Vec<Term<'a>>, String> {
    let connection = database.0.lock().map_err(|_| "an earlier query panicked")?;
    let connection = connection.as_ref().ok_or("this database is closed")?;
    rows(env, connection, sql, &values).map_err(|error| error.to_string())
}

/// Closes the database file now, so its files can be removed.
#[rustler::nif(schedule = "DirtyIo")]
fn close(database: ResourceArc<Database>) -> Atom {
    database.0.lock().map(|mut connection| connection.take()).ok();
    atoms::ok()
}

fn rows<'a>(env: Env<'a>, connection: &Connection, sql: &str, values: &[Term<'a>]) -> rusqlite::Result<Vec<Term<'a>>> {
    let mut statements = Batch::new(connection, sql);
    let Some(mut statement) = statements.next()? else { return Ok(Vec::new()) };
    if statements.next()?.is_some() {
        return Err(rusqlite::Error::MultipleStatement);
    }
    for (index, value) in values.iter().enumerate() {
        statement.raw_bind_parameter(index + 1, bound(*value)?)?;
    }
    let columns: Vec<Term> = statement.column_names().map(|name| name.encode(env)).collect();
    let mut found = Vec::new();
    let mut rows = statement.raw_query();
    while let Some(row) = rows.next()? {
        let values: Vec<Term> =
            (0..columns.len()).map(|column| Ok(read(env, row.get_ref(column)?))).collect::<rusqlite::Result<_>>()?;
        found.push(Term::map_from_term_arrays(env, &columns, &values).expect("as many values as columns"));
    }
    Ok(found)
}

/// An Elixir value as SQLite keeps it: a number, text, `nil`, a boolean as 1 or 0, and `{:blob, binary}` for bytes
/// that aren't text.
fn bound(value: Term) -> rusqlite::Result<Value> {
    let unknown = || rusqlite::Error::ToSqlConversionFailure(format!("SQLite can't keep {value:?}").into());
    Ok(match value.get_type() {
        TermType::Integer => Value::Integer(value.decode().map_err(|_| unknown())?),
        TermType::Float => Value::Real(value.decode().map_err(|_| unknown())?),
        TermType::Binary => Value::Text(value.decode().map_err(|_| unknown())?),
        TermType::Atom if value == nil().to_term(value.get_env()) => Value::Null,
        TermType::Atom if value == true_().to_term(value.get_env()) => Value::Integer(1),
        TermType::Atom if value == false_().to_term(value.get_env()) => Value::Integer(0),
        TermType::Tuple => match value.decode::<(Atom, Binary)>() {
            Ok((tag, bytes)) if tag == atoms::blob() => Value::Blob(bytes.as_slice().to_vec()),
            _ => return Err(unknown()),
        },
        _ => return Err(unknown()),
    })
}

fn read<'a>(env: Env<'a>, value: ValueRef) -> Term<'a> {
    match value {
        ValueRef::Null => nil().encode(env),
        ValueRef::Integer(integer) => integer.encode(env),
        ValueRef::Real(real) => real.encode(env),
        ValueRef::Text(bytes) | ValueRef::Blob(bytes) => {
            let mut binary = NewBinary::new(env, bytes.len());
            binary.as_mut_slice().copy_from_slice(bytes);
            Binary::from(binary).to_term(env)
        }
    }
}

rustler::init!("Elixir.Sqlite");
