//! Snapshot keys: the identity of a prepared step's outputs.
//!
//! A key is the SHA-256 of the canonical JSON of everything that decides the outputs: the recipe
//! and its file, the step after substitution, the base, the platform, the resolved tools, the
//! content of the declared inputs at the locked commit, and the keys of the steps before it.
//! Two workspaces with equal keys may share one snapshot. Field names are part of the key: changing
//! one, or the field order, changes every key, so `KEY_SCHEMA` moves with any such change.

use std::collections::BTreeMap;

use serde::Serialize;
use sha2::{Digest, Sha256};

/// The version of the key document. Bumped whenever its fields or their meaning change.
pub const KEY_SCHEMA: u32 = 1;

/// Everything a snapshot key is made of. Maps and vectors serialize in a fixed order.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct KeyInput {
    pub schema: u32,
    /// `project:<slug>`: snapshots are shared within a project only.
    pub scope: String,
    pub recipe: String,
    pub revision: u32,
    pub recipe_hash: String,
    /// `<repository>/<step id>`.
    pub step: String,
    /// The command after substitution.
    pub run: String,
    pub cwd: String,
    pub outputs: Vec<String>,
    /// Values of the profile variables the step sees.
    pub env: BTreeMap<String, String>,
    /// Names of the secrets granted to the step (never their values).
    pub secrets: Vec<String>,
    /// The base's digest.
    pub base: String,
    pub os: String,
    pub arch: String,
    /// `canonical`, or the real root path of a runner without canonical paths.
    pub layout: String,
    /// Resolved tool versions.
    pub tools: BTreeMap<String, String>,
    /// Input path to blob id, or `{"<tree>": sha}` when the whole repository is the input.
    pub inputs: BTreeMap<String, String>,
    /// Keys of the earlier steps of the same component.
    pub depends: Vec<String>,
}

/// The snapshot key of `input`: lowercase hex SHA-256 of its JSON.
pub fn key(input: &KeyInput) -> String {
    // The input is plain strings, numbers and maps, so serializing it cannot fail.
    let json = serde_json::to_vec(input).expect("a key input always serializes");
    sha256_hex(&json)
}

/// Lowercase hex SHA-256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> KeyInput {
        KeyInput {
            schema: KEY_SCHEMA,
            scope: "project:shop".into(),
            recipe: "node-npm".into(),
            revision: 2,
            recipe_hash: "rh".into(),
            step: "web/install".into(),
            run: "npm ci".into(),
            cwd: String::new(),
            outputs: vec!["node_modules".into()],
            env: BTreeMap::from([("NODE_ENV".to_string(), "production".to_string())]),
            secrets: vec!["npm_token".into()],
            base: "sha256:base".into(),
            os: "linux".into(),
            arch: "x86_64".into(),
            layout: "canonical".into(),
            tools: BTreeMap::from([("node".to_string(), "22.4.0".to_string())]),
            inputs: BTreeMap::from([("package-lock.json".to_string(), "blob1".to_string())]),
            depends: vec!["key0".into()],
        }
    }

    /// The canonical document of `base()`, as the key is computed from it. Changing this literal
    /// is a deliberate change of every key, which needs a `KEY_SCHEMA` bump.
    const GOLDEN_JSON: &str = concat!(
        r#"{"schema":1,"scope":"project:shop","recipe":"node-npm","revision":2,"recipe_hash":"rh","#,
        r#""step":"web/install","run":"npm ci","cwd":"","outputs":["node_modules"],"#,
        r#""env":{"NODE_ENV":"production"},"secrets":["npm_token"],"base":"sha256:base","#,
        r#""os":"linux","arch":"x86_64","layout":"canonical","tools":{"node":"22.4.0"},"#,
        r#""inputs":{"package-lock.json":"blob1"},"depends":["key0"]}"#
    );

    /// `sha256(GOLDEN_JSON)`, computed outside the code with `sha256sum`.
    const GOLDEN_KEY: &str = "e111ec9ece2bbb9b6bbd3e95c7e96dd15f9242c725fcb9ed5341c77e09df38a9";

    #[test]
    fn the_key_of_a_fixed_input_is_a_golden_value() {
        assert_eq!(serde_json::to_string(&base()).unwrap(), GOLDEN_JSON, "the canonical document changed");
        // The SHA-256 of the golden document, computed independently with sha256sum.
        assert_eq!(key(&base()), GOLDEN_KEY);
        assert_eq!(key(&base()).len(), 64);
    }

    #[test]
    fn any_field_change_changes_the_key() {
        let original = key(&base());
        let changes: Vec<(&str, Box<dyn Fn(&mut KeyInput)>)> = vec![
            ("schema", Box::new(|i| i.schema += 1)),
            ("scope", Box::new(|i| i.scope = "project:other".into())),
            ("recipe", Box::new(|i| i.recipe = "rust-cargo".into())),
            ("revision", Box::new(|i| i.revision += 1)),
            ("recipe_hash", Box::new(|i| i.recipe_hash.push('x'))),
            ("step", Box::new(|i| i.step = "web/build".into())),
            ("run", Box::new(|i| i.run = "npm ci --omit=dev".into())),
            ("cwd", Box::new(|i| i.cwd = "front".into())),
            ("outputs", Box::new(|i| i.outputs.push("dist".into()))),
            ("env value", Box::new(|i| {
                i.env.insert("NODE_ENV".into(), "test".into());
            })),
            ("env name", Box::new(|i| {
                i.env.insert("CI".into(), "1".into());
            })),
            ("secrets", Box::new(|i| i.secrets.clear())),
            ("base", Box::new(|i| i.base = "sha256:other".into())),
            ("os", Box::new(|i| i.os = "darwin".into())),
            ("arch", Box::new(|i| i.arch = "aarch64".into())),
            ("layout", Box::new(|i| i.layout = "/home/me/ws".into())),
            ("tools", Box::new(|i| {
                i.tools.insert("node".into(), "22.5.0".into());
            })),
            ("inputs blob", Box::new(|i| {
                i.inputs.insert("package-lock.json".into(), "blob2".into());
            })),
            ("inputs path", Box::new(|i| {
                i.inputs.insert("package.json".into(), "blob1".into());
            })),
            ("depends", Box::new(|i| i.depends.clear())),
        ];
        for (field, change) in changes {
            let mut input = base();
            change(&mut input);
            assert_ne!(key(&input), original, "changing {field} must change the key");
        }
    }

    #[test]
    fn equal_inputs_have_equal_keys_and_order_does_not_matter() {
        let a = base();
        let mut b = base();
        // Maps are ordered by key, so the order they were filled in does not matter.
        b.env = BTreeMap::new();
        b.env.insert("ZZ".into(), "1".into());
        b.env.insert("AA".into(), "2".into());
        let mut c = base();
        c.env = BTreeMap::new();
        c.env.insert("AA".into(), "2".into());
        c.env.insert("ZZ".into(), "1".into());
        assert_eq!(key(&b), key(&c));
        assert_eq!(key(&a), key(&base()));
    }

    #[test]
    fn sha256_hex_matches_the_standard_vector() {
        assert_eq!(sha256_hex(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }
}
