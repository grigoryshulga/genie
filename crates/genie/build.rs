//! Builds the web UI into the binary: the files of `web/dist` (`npm run
//! build:web`) become `$OUT_DIR/web_assets.rs`, source maps left out. A
//! compressible file gets a gzip and a brotli copy under `$OUT_DIR/web`, so the
//! server sends them without compressing anything at run time.
//!
//! - `GENIE_WEB_DIST=<dir>` (relative to the repository root) names the build
//!   and requires it: a release build fails rather than ship without the UI,
//!   and setting the variable re-runs this script.
//! - Otherwise `web/dist` is taken when it is there. A build made before it was
//!   is not redone when it appears (Cargo would re-run the script on every build
//!   while a watched path is missing); such a binary serves `web/dist` of its
//!   working directory, or the directory given with `--web`.

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use flate2::Compression;
use flate2::write::GzEncoder;

/// Files under this size are not worth a second copy in the binary.
const COMPRESS_ABOVE_BYTES: usize = 1024;
/// A copy is kept only when it saves at least this much of the original.
const KEEP_BELOW_PERCENT: usize = 95;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=GENIE_WEB_DIST");
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let root = manifest.join("../..");
    let explicit = std::env::var_os("GENIE_WEB_DIST").filter(|d| !d.is_empty()).map(|d| root.join(d));
    let dist = explicit.clone().unwrap_or_else(|| root.join("web/dist"));
    let mut files = Vec::new();
    if dist.join("index.html").is_file() {
        println!("cargo:rerun-if-changed={}", dist.display());
        let dist = dist.canonicalize().expect("web/dist");
        collect(&dist, &dist, &mut files);
        files.sort();
    } else if let Some(d) = explicit {
        panic!("GENIE_WEB_DIST={}: no index.html there; build the web UI first (npm run build:web)", d.display());
    }
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let mut out = String::from(
        "/// The files of the built web UI: path under `web/dist`, contents, and the\n/// precompressed copies worth their bytes.\npub static WEB_ASSETS: &[WebAsset] = &[\n",
    );
    for (rel, file) in &files {
        let raw = std::fs::read(file).expect("web/dist file");
        let (gzip, br) = if compressible(rel) && raw.len() > COMPRESS_ABOVE_BYTES {
            let gzip = packed(&out_dir, rel, &raw, "gz", gzip);
            let br = packed(&out_dir, rel, &raw, "br", brotli);
            (gzip, br)
        } else {
            (None, None)
        };
        writeln!(
            out,
            "    WebAsset {{ path: {rel:?}, raw: include_bytes!({:?}), gzip: {}, br: {} }},",
            file.display().to_string(),
            variant(gzip.as_deref()),
            variant(br.as_deref())
        )
        .unwrap();
    }
    out.push_str("];\n");
    write_if_changed(&out_dir.join("web_assets.rs"), out.as_bytes());
}

/// What a compressor may make smaller: text formats that shrink well, which the
/// browser can be asked to take compressed.
fn compressible(rel: &str) -> bool {
    matches!(rel.rsplit_once('.').map(|(_, ext)| ext), Some("html" | "js" | "mjs" | "css" | "svg" | "json" | "txt"))
}

/// A compressed copy of `raw` written next to the generated asset list, or
/// `None` when the copy is not at least 5% smaller than `raw`: a saving below
/// that does not pay for the bytes in the binary. Its path feeds `include_bytes!`.
fn packed(out_dir: &Path, rel: &str, raw: &[u8], ext: &str, compress: fn(&[u8]) -> Vec<u8>) -> Option<PathBuf> {
    let data = compress(raw);
    if data.len() * 100 > raw.len() * KEEP_BELOW_PERCENT {
        return None;
    }
    let path = out_dir.join("web").join(format!("{rel}.{ext}"));
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).expect("precompressed assets");
    }
    write_if_changed(&path, &data);
    Some(path)
}

fn gzip(raw: &[u8]) -> Vec<u8> {
    let mut enc = GzEncoder::new(Vec::new(), Compression::best());
    enc.write_all(raw).expect("gzip");
    enc.finish().expect("gzip")
}

fn brotli(raw: &[u8]) -> Vec<u8> {
    let params = brotli::enc::BrotliEncoderParams { quality: 11, lgwin: 22, ..Default::default() };
    let mut out = Vec::new();
    brotli::BrotliCompress(&mut &raw[..], &mut out, &params).expect("brotli");
    out
}

/// The `include_bytes!` of a copy, or `None` for a file left whole.
fn variant(path: Option<&Path>) -> String {
    path.map_or_else(|| "None".into(), |p| format!("Some(include_bytes!({:?}))", p.display().to_string()))
}

/// Keep an unchanged output: rewriting it would recompile the crate for nothing.
fn write_if_changed(path: &Path, data: &[u8]) {
    if std::fs::read(path).ok().as_deref() != Some(data) {
        std::fs::write(path, data).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    }
}

fn collect(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir).expect("web/dist").filter_map(Result::ok).collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let path = e.path();
        if path.is_dir() {
            collect(root, &path, out);
        } else if path.extension().is_none_or(|x| x != "map") {
            let rel =
                path.strip_prefix(root).expect("under web/dist").components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>();
            out.push((rel.join("/"), path));
        }
    }
}
