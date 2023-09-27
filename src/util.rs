use std::collections::VecDeque;
use std::env;
use std::fs::read_to_string;
use std::path::Path;

use toml::value::Value;
use toml::Table;

pub fn get_env_var(key: &str) -> String {
  for (k, v) in env::vars() {
    if k == key {
      return v;
    }
  }

  panic!("Need env var: {}", key);
}

pub fn get_toml_entry<P, V, S>(toml_file: P, path: V) -> Option<Value>
where
  P: AsRef<Path>,
  V: Into<VecDeque<S>>,
  S: ToString,
{
  let toml_content = read_to_string(toml_file).expect("unable to read toml file");
  let mut table = toml_content.parse::<Table>().expect("invalid toml content");

  let mut path = path.into();
  if path.is_empty() {
    return Some(Value::Table(table));
  }

  loop {
    let next_sub_path = path.pop_front().unwrap().to_string();
    let value = table.get(&next_sub_path)?.clone();

    if path.is_empty() {
      return Some(value);
    }

    if let Value::Table(t) = value {
      table = t;
    } else {
      return None;
    }
  }
}

pub fn get_toml_string<P, V, S>(toml_file: P, path: V) -> Option<String>
where
  P: AsRef<Path>,
  V: Into<VecDeque<S>>,
  S: ToString,
{
  match get_toml_entry(toml_file, path) {
    Some(Value::String(s)) => Some(s),
    _ => None,
  }
}

pub fn get_toml_string_vec<P, V, S>(toml_file: P, path: V) -> Option<Vec<String>>
where
  P: AsRef<Path>,
  V: Into<VecDeque<S>>,
  S: ToString,
{
  match get_toml_entry(toml_file, path) {
    Some(Value::Array(a)) => a
      .into_iter()
      .map(|v| match v {
        Value::String(s) => Some(s),
        _ => None,
      })
      .collect(),
    _ => None,
  }
}
