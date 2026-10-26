//! Request validation with the same rules and error shape the API has always had:
//! `400 {"error": "Invalid request", "details": [{"path", "message"}]}`.

use std::collections::{BTreeSet, HashMap};

use serde::Serialize;
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::error::AppError;

#[derive(Debug, Clone, Serialize)]
pub struct Issue {
    pub path: String,
    pub message: String,
}

impl Issue {
    pub fn new(path: &str, message: impl Into<String>) -> Self {
        Issue { path: path.to_string(), message: message.into() }
    }
}

/// A field that may be left out (`Absent`), sent as `null`, or sent with a value.
#[derive(Debug, Clone, PartialEq)]
pub enum Opt<T> {
    Absent,
    Null,
    Val(T),
}

impl<T> Opt<T> {
    pub fn is_absent(&self) -> bool {
        matches!(self, Opt::Absent)
    }
    pub fn is_present(&self) -> bool {
        !self.is_absent()
    }
    pub fn value(&self) -> Option<&T> {
        match self {
            Opt::Val(v) => Some(v),
            _ => None,
        }
    }
    pub fn into_option(self) -> Option<T> {
        match self {
            Opt::Val(v) => Some(v),
            _ => None,
        }
    }
}

impl Opt<String> {
    pub fn to_json(&self) -> Option<Value> {
        match self {
            Opt::Absent => None,
            Opt::Null => Some(Value::Null),
            Opt::Val(v) => Some(Value::String(v.clone())),
        }
    }
}

pub fn type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

pub fn parse_uuid(s: &str) -> Option<Uuid> {
    if s.len() != 36 {
        return None;
    }
    Uuid::parse_str(s).ok()
}

/// Validates a path parameter that must be a UUID.
pub fn path_uuid(s: &str) -> Result<Uuid, AppError> {
    parse_uuid(s).ok_or_else(|| AppError::Invalid(vec![Issue::new("", "Invalid uuid")]))
}

pub fn is_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10 && b.iter().enumerate().all(|(i, c)| if i == 4 || i == 7 { *c == b'-' } else { c.is_ascii_digit() })
}

pub fn is_email(s: &str) -> bool {
    let Some((local, domain)) = s.rsplit_once('@') else { return false };
    let ok_local = !local.is_empty()
        && !local.starts_with('.')
        && !local.ends_with('.')
        && !local.contains("..")
        && local.chars().all(|c| c.is_ascii_alphanumeric() || "._+-'!#$%&*/=?^`{|}~".contains(c));
    let labels: Vec<&str> = domain.split('.').collect();
    let ok_domain = labels.len() >= 2
        && labels.iter().all(|l| {
            !l.is_empty()
                && !l.starts_with('-')
                && !l.ends_with('-')
                && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
        && labels.last().map(|l| l.len() >= 2 && l.chars().all(|c| c.is_ascii_alphabetic())).unwrap_or(false);
    ok_local && ok_domain
}

fn quoted(list: &[&str]) -> String {
    list.iter().map(|v| format!("'{v}'")).collect::<Vec<_>>().join(" | ")
}

/// Rules for a string field.
#[derive(Clone, Copy)]
pub struct Str {
    pub min: usize,
    pub max: usize,
    pub optional: bool,
    pub nullable: bool,
    pub trim: bool,
    /// An empty string (after trimming) becomes `null`.
    pub empty_null: bool,
    pub lower: bool,
    pub check: Option<fn(&str) -> Option<&'static str>>,
}

impl Str {
    /// Required, trimmed, between `min` and `max` characters.
    pub const fn req(min: usize, max: usize) -> Str {
        Str { min, max, optional: false, nullable: false, trim: true, empty_null: false, lower: false, check: None }
    }
    /// May be left out, but not null.
    pub const fn optional(self) -> Str {
        Str { optional: true, ..self }
    }
    /// May be left out or null.
    pub const fn nullish(self) -> Str {
        Str { optional: true, nullable: true, ..self }
    }
    /// Optional free text: blank becomes null.
    pub const fn text(max: usize) -> Str {
        Str { min: 0, max, optional: true, nullable: true, trim: true, empty_null: true, lower: false, check: None }
    }
    pub const fn lower(self) -> Str {
        Str { lower: true, ..self }
    }
    pub const fn check(self, f: fn(&str) -> Option<&'static str>) -> Str {
        Str { check: Some(f), ..self }
    }
}

pub fn email_check(s: &str) -> Option<&'static str> {
    (!is_email(s)).then_some("Invalid email")
}

pub fn date_check(s: &str) -> Option<&'static str> {
    (!is_date(s)).then_some("Use YYYY-MM-DD")
}

pub fn uuid_check(s: &str) -> Option<&'static str> {
    parse_uuid(s).is_none().then_some("Invalid uuid")
}

pub const EMAIL: Str = Str::req(0, 254).lower().check(email_check);
pub const UUID: Str = Str { trim: false, ..Str::req(0, usize::MAX).check(uuid_check) };
pub const DATE: Str = Str { trim: false, ..Str::req(0, usize::MAX).nullish().check(date_check) };

pub struct Input {
    map: Map<String, Value>,
    issues: Vec<Issue>,
    known: BTreeSet<String>,
}

impl Input {
    /// Starts validating a JSON body, which must be an object.
    pub fn body(value: &Value) -> Input {
        match value {
            Value::Object(map) => Input { map: map.clone(), issues: vec![], known: BTreeSet::new() },
            other => Input {
                map: Map::new(),
                issues: vec![Issue::new("", format!("Expected object, received {}", type_name(other)))],
                known: BTreeSet::new(),
            },
        }
    }

    pub fn query(params: &HashMap<String, String>) -> Input {
        let map = params.iter().map(|(k, v)| (k.clone(), Value::String(v.clone()))).collect();
        Input { map, issues: vec![], known: BTreeSet::new() }
    }

    fn raw(&mut self, key: &str) -> Option<Value> {
        self.known.insert(key.to_string());
        self.map.get(key).cloned()
    }

    pub fn fail(&mut self, path: &str, message: impl Into<String>) {
        self.issues.push(Issue::new(path, message));
    }

    pub fn str(&mut self, key: &str, rule: Str) -> Opt<String> {
        let raw = self.raw(key);
        let s = match raw {
            None if rule.optional => return Opt::Absent,
            None => {
                self.fail(key, "Required");
                return Opt::Absent;
            }
            Some(Value::Null) if rule.nullable => return Opt::Null,
            Some(Value::String(s)) => s,
            Some(other) => {
                self.fail(key, format!("Expected string, received {}", type_name(&other)));
                return Opt::Absent;
            }
        };
        let mut s = if rule.trim { s.trim().to_string() } else { s };
        if rule.lower {
            s = s.to_lowercase();
        }
        let len = s.chars().count();
        if len < rule.min {
            self.fail(key, format!("String must contain at least {} character(s)", rule.min));
            return Opt::Absent;
        }
        if len > rule.max {
            self.fail(key, format!("String must contain at most {} character(s)", rule.max));
            return Opt::Absent;
        }
        if rule.empty_null && s.is_empty() {
            return Opt::Null;
        }
        if let Some(message) = rule.check.and_then(|f| f(&s)) {
            self.fail(key, message);
            return Opt::Absent;
        }
        Opt::Val(s)
    }

    /// A required string; returns an empty string when invalid (the request then fails anyway).
    pub fn req(&mut self, key: &str, rule: Str) -> String {
        self.str(key, rule).into_option().unwrap_or_default()
    }

    /// One value from a fixed list. Not trimmed.
    pub fn one_of(&mut self, key: &str, list: &[&str], optional: bool, nullable: bool) -> Opt<String> {
        match self.raw(key) {
            None if optional => Opt::Absent,
            None => {
                self.fail(key, "Required");
                Opt::Absent
            }
            Some(Value::Null) if nullable => Opt::Null,
            Some(Value::String(s)) if list.contains(&s.as_str()) => Opt::Val(s),
            Some(Value::String(s)) => {
                self.fail(key, format!("Invalid enum value. Expected {}, received '{s}'", quoted(list)));
                Opt::Absent
            }
            Some(other) => {
                self.fail(key, format!("Expected {}, received {}", quoted(list), type_name(&other)));
                Opt::Absent
            }
        }
    }

    pub fn one_of_or(&mut self, key: &str, list: &[&str], default: &str) -> String {
        match self.one_of(key, list, true, false) {
            Opt::Val(v) => v,
            _ => default.to_string(),
        }
    }

    pub fn uuid(&mut self, key: &str, optional: bool, nullable: bool) -> Opt<Uuid> {
        let rule = Str { optional, nullable, ..UUID };
        match self.str(key, rule) {
            Opt::Val(s) => Opt::Val(parse_uuid(&s).unwrap()),
            Opt::Null => Opt::Null,
            Opt::Absent => Opt::Absent,
        }
    }

    pub fn boolean(&mut self, key: &str, nullable: bool) -> Opt<bool> {
        match self.raw(key) {
            None => Opt::Absent,
            Some(Value::Null) if nullable => Opt::Null,
            Some(Value::Bool(b)) => Opt::Val(b),
            Some(other) => {
                self.fail(key, format!("Expected boolean, received {}", type_name(&other)));
                Opt::Absent
            }
        }
    }

    pub fn boolean_or(&mut self, key: &str, default: bool) -> bool {
        self.boolean(key, false).into_option().unwrap_or(default)
    }

    /// A non-negative amount of money, as a number or a decimal string with at most two decimals.
    pub fn money(&mut self, key: &str) -> Opt<f64> {
        let n = match self.raw(key) {
            None => return Opt::Absent,
            Some(Value::Null) => return Opt::Null,
            Some(Value::Number(n)) => n.as_f64().unwrap_or(f64::NAN),
            Some(Value::String(s)) => {
                let t = s.trim();
                let valid = match t.split_once('.') {
                    Some((a, b)) => {
                        !a.is_empty()
                            && a.bytes().all(|c| c.is_ascii_digit())
                            && (1..=2).contains(&b.len())
                            && b.bytes().all(|c| c.is_ascii_digit())
                    }
                    None => !t.is_empty() && t.bytes().all(|c| c.is_ascii_digit()),
                };
                if !valid {
                    self.fail(key, "Invalid");
                    return Opt::Absent;
                }
                t.parse().unwrap()
            }
            Some(other) => {
                self.fail(key, format!("Expected number, received {}", type_name(&other)));
                return Opt::Absent;
            }
        };
        if !(0.0..1e10).contains(&n) {
            self.fail(key, "Must be a positive amount");
            return Opt::Absent;
        }
        Opt::Val(n)
    }

    /// A whole number from a query string, with bounds and a default.
    pub fn int(&mut self, key: &str, min: i64, max: i64, default: i64) -> i64 {
        let Some(raw) = self.raw(key) else { return default };
        let text = match &raw {
            Value::String(s) => s.trim().to_string(),
            other => other.to_string(),
        };
        let n: f64 = if text.is_empty() { 0.0 } else { text.parse().unwrap_or(f64::NAN) };
        if n.is_nan() {
            self.fail(key, "Expected number, received nan");
        } else if n.fract() != 0.0 {
            self.fail(key, "Expected integer, received float");
        } else if n < min as f64 {
            self.fail(key, format!("Number must be greater than or equal to {min}"));
        } else if n > max as f64 {
            self.fail(key, format!("Number must be less than or equal to {max}"));
        } else {
            return n as i64;
        }
        default
    }

    /// A nested object, validated by `f`.
    pub fn object(&mut self, key: &str, f: impl FnOnce(&mut Input)) -> Opt<()> {
        match self.raw(key) {
            None => Opt::Absent,
            Some(Value::Object(map)) => {
                let mut inner = Input { map, issues: vec![], known: BTreeSet::new() };
                f(&mut inner);
                inner.reject_unknown();
                for mut issue in inner.issues {
                    issue.path = if issue.path.is_empty() { key.to_string() } else { format!("{key}.{}", issue.path) };
                    self.issues.push(issue);
                }
                Opt::Val(())
            }
            Some(other) => {
                self.fail(key, format!("Expected object, received {}", type_name(&other)));
                Opt::Absent
            }
        }
    }

    pub fn raw_value(&mut self, key: &str) -> Option<Value> {
        self.raw(key)
    }

    fn reject_unknown(&mut self) {
        let unknown: Vec<String> =
            self.map.keys().filter(|k| !self.known.contains(*k)).map(|k| format!("'{k}'")).collect();
        if !unknown.is_empty() {
            self.fail("", format!("Unrecognized key(s) in object: {}", unknown.join(", ")));
        }
    }

    /// Finishes validation, refusing fields the schema does not know.
    pub fn strict(mut self) -> Result<(), AppError> {
        self.reject_unknown();
        self.done()
    }

    /// Finishes validation, ignoring fields the schema does not know.
    pub fn done(self) -> Result<(), AppError> {
        if self.issues.is_empty() {
            Ok(())
        } else {
            Err(AppError::Invalid(self.issues))
        }
    }

    #[cfg(test)]
    pub fn into_issues(self) -> Vec<Issue> {
        self.issues
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn emails() {
        assert!(is_email("pat.lee+it@acme.co"));
        assert!(!is_email("pat@acme"));
        assert!(!is_email("@acme.com"));
        assert!(!is_email("pat lee@acme.com"));
    }

    #[test]
    fn dates_and_uuids() {
        assert!(is_date("2025-02-01"));
        assert!(!is_date("2025-2-01"));
        assert!(parse_uuid("4b0c7a52-7cb0-4f58-9d43-4a0c0d1fbb7e").is_some());
        assert!(parse_uuid("4b0c7a527cb04f589d434a0c0d1fbb7e").is_none());
    }

    #[test]
    fn optional_text_and_strictness() {
        let mut i = Input::body(&json!({ "notes": "  ", "name": " A ", "extra": 1 }));
        assert_eq!(i.str("notes", Str::text(10)), Opt::Null);
        assert_eq!(i.req("name", Str::req(1, 5)), "A");
        assert_eq!(i.str("serial", Str::text(10)), Opt::Absent);
        let err = i.strict().unwrap_err();
        assert!(matches!(err, AppError::Invalid(ref v) if v[0].message.contains("'extra'")));
    }

    #[test]
    fn enums_and_money() {
        let mut i = Input::body(&json!({ "type": "Toaster", "cost": "42.50", "bad": "4.555" }));
        assert!(i.one_of("type", &["Laptop"], false, false).is_absent());
        assert_eq!(i.money("cost"), Opt::Val(42.5));
        assert!(i.money("bad").is_absent());
        assert_eq!(i.into_issues().len(), 2);
    }
}
