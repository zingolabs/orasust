#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};
use typify::{MapType, TypeSpace, TypeSpaceSettings};

const SCHEMA_DIR: &str = "schema";
const ROOT_SCHEMA: &str = "image-manifest-schema.json";
const REFERENCED_SCHEMAS: [&str; 3] = [
    "content-descriptor.json",
    "defs-descriptor.json",
    "defs.json",
];
const ROOT_TITLE: &str = "ImageManifest";
const GENERATED: &str = "image_spec.rs";
const DEFINITIONS: &str = "definitions";
const DEFINITIONS_POINTER: &str = "#/definitions/";
const REF: &str = "$ref";
const TITLE: &str = "title";
const DRAFT_ID: &str = "id";
const DRAFT_SCHEMA: &str = "$schema";
const FRAGMENT: char = '#';
const DEFINITION_JOIN: char = '_';
const MAP_TYPE: &str = "::std::collections::BTreeMap";

fn definition_key(file: &str, definition: Option<&str>) -> String {
    let stem = Path::new(file)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or(file)
        .replace('-', "_");
    match definition {
        Some(name) => format!("{stem}{DEFINITION_JOIN}{name}"),
        None => stem,
    }
}

fn rewritten_reference(reference: &str, current_file: &str) -> String {
    let (file, fragment) = reference.split_once(FRAGMENT).unwrap_or((reference, ""));
    let file = if file.is_empty() { current_file } else { file };
    match fragment.strip_prefix(DEFINITIONS_POINTER.trim_start_matches(FRAGMENT)) {
        Some(name) => format!("{DEFINITIONS_POINTER}{}", definition_key(file, Some(name))),
        None if fragment.is_empty() => {
            format!("{DEFINITIONS_POINTER}{}", definition_key(file, None))
        }
        None => panic!("unsupported reference {reference} in {current_file}"),
    }
}

fn rewrite(node: Value, current_file: &str, is_root: bool) -> Value {
    match node {
        Value::Object(object) => Value::Object(
            object
                .into_iter()
                .filter(|(key, _)| is_root || (key != DRAFT_ID && key != DRAFT_SCHEMA))
                .map(|(key, value)| match (key.as_str(), &value) {
                    (REF, Value::String(reference)) => (
                        key,
                        Value::String(rewritten_reference(reference, current_file)),
                    ),
                    _ => (key, rewrite(value, current_file, is_root)),
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .map(|item| rewrite(item, current_file, is_root))
                .collect(),
        ),
        other => other,
    }
}

fn read_schema(dir: &Path, file: &str) -> Map<String, Value> {
    let path = dir.join(file);
    println!("cargo:rerun-if-changed={}", path.display());
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    match serde_json::from_str(&text) {
        Ok(Value::Object(object)) => object,
        _ => panic!("{} is not a JSON object", path.display()),
    }
}

fn bundled(dir: &Path) -> Value {
    let mut definitions = Map::new();
    for file in REFERENCED_SCHEMAS {
        let mut document = read_schema(dir, file);
        if let Some(Value::Object(own)) = document.remove(DEFINITIONS) {
            for (name, definition) in own {
                definitions.insert(
                    definition_key(file, Some(&name)),
                    rewrite(definition, file, false),
                );
            }
        }
        definitions.insert(
            definition_key(file, None),
            rewrite(Value::Object(document), file, false),
        );
    }
    let mut root = read_schema(dir, ROOT_SCHEMA);
    if let Some(Value::Object(own)) = root.remove(DEFINITIONS) {
        for (name, definition) in own {
            definitions.insert(
                definition_key(ROOT_SCHEMA, Some(&name)),
                rewrite(definition, ROOT_SCHEMA, false),
            );
        }
    }
    root.insert(TITLE.to_string(), json!(ROOT_TITLE));
    let mut root = rewrite(Value::Object(root), ROOT_SCHEMA, true);
    root[DEFINITIONS] = Value::Object(definitions);
    root
}

fn main() {
    let schema_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(SCHEMA_DIR);
    let schema: schemars::schema::RootSchema = serde_json::from_value(bundled(&schema_dir))
        .expect("the bundled image-spec schema parses as a JSON Schema");
    let mut settings = TypeSpaceSettings::default();
    settings.with_map_type(MapType::new(MAP_TYPE));
    let mut type_space = TypeSpace::new(&settings);
    type_space
        .add_root_schema(schema)
        .expect("the image-spec schema converts to Rust types");
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR is set")).join(GENERATED);
    std::fs::write(&out, type_space.to_stream().to_string())
        .unwrap_or_else(|e| panic!("cannot write {}: {e}", out.display()));
}
