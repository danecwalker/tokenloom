//! Tiny dotted-path accessor for declarative JSON extraction:
//! `a.b.0.c` — object keys by name, array elements by index.

use serde_json::Value;

pub fn get<'a>(v: &'a Value, path: &str) -> Option<&'a Value> {
    let mut cur = v;
    for seg in path.split('.') {
        cur = match cur {
            Value::Object(map) => map.get(seg)?,
            Value::Array(arr) => arr.get(seg.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(cur)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn navigates_objects_arrays_and_indices() {
        let v = json!({"a": {"b": [{"c": 1}, {"c": 2}]}});
        assert_eq!(get(&v, "a.b.1.c"), Some(&json!(2)));
        assert_eq!(get(&v, "a.b.5.c"), None);
        assert_eq!(get(&v, "a.z"), None);
    }
}
