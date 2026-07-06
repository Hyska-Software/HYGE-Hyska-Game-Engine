//! R-041 IBL environment import integration test.
//!
//! Validates the offline environment bake path:
//!
//! 1. A small synthetic Radiance HDR is imported via
//!    `import_environment`.
//! 2. The output `.hyge-env` is content-addressed by BLAKE3.
//! 3. The same import is re-run; the hashes are stable.
//! 4. A modified HDR produces a different hash.
//! 5. The `import_environment` path is reachable from
//!    `hyge-tools import` (tested in `hyge-tools` unit tests).

use std::io::Write;
use std::path::PathBuf;

use hyge_asset::prelude::*;
use hyge_render::ibl::BakeConfig;

fn unique_temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "hyge-asset-r041-{}-{}-{}",
        label,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("temp dir should be creatable");
    dir
}

/// Writes a minimal valid Radiance RGBE `.hdr` file.
/// Resolution is 4x2 (equirectangular) so the bake is fast.
fn synthetic_hdr_bytes() -> Vec<u8> {
    // Minimal Radiance HDR header + 4x2 RGBE pixels.
    // rgbe crate accepts headers like this.
    let mut buf = Vec::new();
    writeln!(buf, "#?RADIANCE").unwrap();
    writeln!(buf, "FORMAT=32-bit_rle_rgbe").unwrap();
    writeln!(buf).unwrap();
    writeln!(buf, "-Y 2 +X 4").unwrap();
    // 8 pixels * 4 bytes
    for _ in 0..8 {
        buf.extend_from_slice(&[128, 128, 128, 128]);
    }
    buf
}

fn small_bake_config() -> BakeConfig {
    BakeConfig {
        prefilter_size: 32,
        irradiance_size: 8,
        brdf_lut_size: 8,
        sample_count: 32,
    }
}

#[test]
fn import_hdr_creates_hyge_env() {
    let dir = unique_temp_dir("basic");
    let hdr = dir.join("sky.hdr");
    std::fs::write(&hdr, synthetic_hdr_bytes()).unwrap();

    let out = dir.join("cook");
    let report = import_environment_with_config(&hdr, &out, small_bake_config())
        .expect("import_environment must succeed");

    let env_path = out.join(format!("{}.hyge-env", report.env_hash));
    assert!(
        env_path.is_file(),
        ".hyge-env file should exist at {}",
        env_path.display()
    );
    assert!(!report.source_hash.is_empty());
    assert!(!report.env_hash.is_empty());
    assert!(report.prefilter_size > 0);
    assert!(report.prefilter_mips > 0);
    assert!(report.irradiance_size > 0);
    assert!(report.brdf_lut_size > 0);
}

#[test]
fn import_hdr_is_hash_stable() {
    let dir = unique_temp_dir("stable");
    let hdr = dir.join("sky.hdr");
    std::fs::write(&hdr, synthetic_hdr_bytes()).unwrap();

    let out = dir.join("cook");
    let report_a =
        import_environment_with_config(&hdr, &out, small_bake_config()).expect("first import");
    let report_b =
        import_environment_with_config(&hdr, &out, small_bake_config()).expect("second import");

    assert_eq!(report_a.source_hash, report_b.source_hash);
    assert_eq!(report_a.env_hash, report_b.env_hash);
}

#[test]
fn modified_hdr_produces_different_env_hash() {
    let dir = unique_temp_dir("modified");
    let hdr_a = dir.join("sky.hdr");
    let mut bytes = synthetic_hdr_bytes();
    std::fs::write(&hdr_a, &bytes).unwrap();

    let out = dir.join("cook");
    let report_a =
        import_environment_with_config(&hdr_a, &out, small_bake_config()).expect("first import");

    // Perturb one pixel; source hash changes, env hash changes.
    let last = bytes.len() - 1;
    bytes[last] = bytes[last].wrapping_add(1);
    let hdr_b = dir.join("sky2.hdr");
    std::fs::write(&hdr_b, &bytes).unwrap();

    let report_b =
        import_environment_with_config(&hdr_b, &out, small_bake_config()).expect("second import");

    assert_ne!(report_a.source_hash, report_b.source_hash);
    assert_ne!(report_a.env_hash, report_b.env_hash);
}

#[test]
fn import_hdr_records_in_asset_db() {
    let dir = unique_temp_dir("db");
    let hdr = dir.join("sky.hdr");
    std::fs::write(&hdr, synthetic_hdr_bytes()).unwrap();

    let out = dir.join("cook");
    std::fs::create_dir_all(&out).unwrap();
    let db_path = out.join(".hyge.db");
    let mut db = AssetDb::open(&db_path).expect("open db");
    let report =
        import_environment_with_config_and_db(&hdr, &out, small_bake_config(), Some(&mut db))
            .expect("import_environment");

    let env_id = AssetId::from(blake3::hash(report.env_hash.as_bytes()));
    let path = db.lookup(&env_id).expect("env recorded in db");
    assert_eq!(path, out.join(format!("{}.hyge-env", report.env_hash)));
}

#[test]
fn import_exr_is_rejected_with_unsupported() {
    let dir = unique_temp_dir("exr");
    let exr = dir.join("sky.exr");
    std::fs::write(&exr, b"not a real exr").unwrap();
    let out = dir.join("cook");

    let err = import_environment_with_config(&exr, &out, small_bake_config())
        .expect_err("EXR must be unsupported");
    let msg = format!("{err}");
    assert!(
        msg.contains("EXR environment maps are not yet supported"),
        "unexpected error: {msg}"
    );
}
