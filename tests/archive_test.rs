//! End-to-end integration tests that drive the real `nextar` binary.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_nextar"))
}

fn run(args: &[&str]) -> Output {
    bin().args(args).output().expect("failed to spawn nextar")
}

fn ok(output: &Output) {
    assert!(
        output.status.success(),
        "command failed: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout),
    );
}

fn fail(output: &Output) {
    assert!(
        !output.status.success(),
        "command unexpectedly succeeded: {}",
        String::from_utf8_lossy(&output.stdout),
    );
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// Build a deterministic test tree under `root`.
fn make_tree(root: &Path) {
    let sub = root.join("sub");
    fs::create_dir_all(&sub).unwrap();

    // compressible text (multiple chunks)
    let mut text = String::new();
    for i in 0..40000 {
        text.push_str(&format!("the quick brown fox jumps over the lazy dog {i}\n"));
    }
    fs::write(root.join("big.txt"), &text).unwrap();

    // incompressible binary
    let mut rng_state = 0x1234_5678_9abc_def0u64;
    let mut bytes = Vec::with_capacity(2 * 1024 * 1024);
    for _ in 0..(2 * 1024 * 1024 / 8) {
        rng_state ^= rng_state << 13;
        rng_state ^= rng_state >> 7;
        rng_state ^= rng_state << 17;
        bytes.extend_from_slice(&rng_state.to_le_bytes());
    }
    fs::write(root.join("rand.bin"), &bytes).unwrap();

    // empty file
    fs::write(root.join("empty.txt"), b"").unwrap();

    // nested
    fs::write(sub.join("nested.txt"), b"hello nested world\n".repeat(1000)).unwrap();
}

/// Recursively compare two trees byte-for-byte (files + dirs + symlinks).
fn compare_trees(a: &Path, b: &Path) {
    let walk = |p: &Path| -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = walkdir::WalkDir::new(p)
            .into_iter()
            .filter_map(|e| e.ok())
            .map(|e| e.path().to_path_buf())
            .collect();
        v.sort();
        v
    };
    let fa = walk(a);
    let fb = walk(b);
    assert_eq!(
        fa.iter().map(|p| p.strip_prefix(a).unwrap()).collect::<Vec<_>>(),
        fb.iter().map(|p| p.strip_prefix(b).unwrap()).collect::<Vec<_>>(),
        "tree structure differs"
    );
    for (pa, pb) in fa.iter().zip(fb.iter()) {
        let ta = fs::symlink_metadata(pa).unwrap();
        let tb = fs::symlink_metadata(pb).unwrap();
        assert_eq!(ta.file_type().is_symlink(), tb.file_type().is_symlink());
        if ta.file_type().is_file() {
            assert_eq!(fs::read(pa).unwrap(), fs::read(pb).unwrap(), "content differs: {pa:?}");
        }
    }
}

fn corrupt_file(path: &Path, offset: u64, bytes: &[u8]) {
    let mut data = fs::read(path).unwrap();
    let start = offset as usize;
    for (i, b) in bytes.iter().enumerate() {
        data[start + i] = *b;
    }
    fs::write(path, &data).unwrap();
}

fn truncate_file(path: &Path, len: u64) {
    let data = fs::read(path).unwrap();
    fs::write(path, &data[..len as usize]).unwrap();
}

#[test]
fn roundtrip_zstd() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(&src).unwrap();
    make_tree(&src);
    let arch = dir.path().join("backup.next");
    let out = dir.path().join("out");

    ok(&run(&["create", src.to_str().unwrap(), "-o", arch.to_str().unwrap(), "-f", "-q"]));
    ok(&run(&["verify", arch.to_str().unwrap(), "-q"]));
    ok(&run(&["list", arch.to_str().unwrap()]));
    ok(&run(&["extract", arch.to_str().unwrap(), "-o", out.to_str().unwrap(), "-q"]));

    compare_trees(&src, &out.join("src"));
}

#[test]
fn roundtrip_lzma2() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(&src).unwrap();
    make_tree(&src);
    let arch = dir.path().join("lz.next");
    let out = dir.path().join("out");

    ok(&run(&["create", src.to_str().unwrap(), "-o", arch.to_str().unwrap(), "-c", "lzma2", "-l", "6", "-f", "-q"]));
    ok(&run(&["verify", arch.to_str().unwrap(), "-q"]));
    ok(&run(&["extract", arch.to_str().unwrap(), "-o", out.to_str().unwrap(), "-q"]));
    compare_trees(&src, &out.join("src"));
}

#[test]
fn multi_block_chunks() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(&src).unwrap();
    make_tree(&src); // ~2 MiB of binary + text
    let arch = dir.path().join("mb.next");
    let out = dir.path().join("out");

    ok(&run(&["create", src.to_str().unwrap(), "-o", arch.to_str().unwrap(), "-b", "256K", "-f", "-q"]));
    let info = run(&["info", arch.to_str().unwrap()]);
    ok(&info);
    // 2 MiB / 256 KiB + text chunks => several blocks
    assert!(String::from_utf8_lossy(&info.stdout).contains("data blocks      :"));
    ok(&run(&["extract", arch.to_str().unwrap(), "-o", out.to_str().unwrap(), "-q"]));
    compare_trees(&src, &out.join("src"));
}

#[test]
fn encrypted_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(&src).unwrap();
    make_tree(&src);
    let arch = dir.path().join("enc.next");
    let out = dir.path().join("out");

    ok(&run(&["create", src.to_str().unwrap(), "-o", arch.to_str().unwrap(), "-p", "hunter2", "-f", "-q"]));

    // No plaintext leakage.
    let data = fs::read(&arch).unwrap();
    assert!(!data.windows(b"quick brown fox".len()).any(|w| w == b"quick brown fox"));

    // Wrong password rejected.
    let bad = run(&["extract", arch.to_str().unwrap(), "-o", out.to_str().unwrap(), "-p", "wrong", "-q"]);
    fail(&bad);

    // Right password works.
    ok(&run(&["extract", arch.to_str().unwrap(), "-o", out.to_str().unwrap(), "-p", "hunter2", "-q"]));
    compare_trees(&src, &out.join("src"));
}

#[test]
fn recovery_repairs_corruption() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(&src).unwrap();
    make_tree(&src);
    let arch = dir.path().join("rec.next");
    let vol = dir.path().join("rec.next.nvol");

    ok(&run(&["create", src.to_str().unwrap(), "-o", arch.to_str().unwrap(), "-r", "8", "-f", "-q"]));
    assert!(vol.exists(), "recovery volume missing");

    // Corrupt bytes inside the data region (past the 60-byte header).
    let arch_size = fs::metadata(&arch).unwrap().len();
    corrupt_file(&arch, 200_000, &[0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x11, 0x22, 0x33]);
    corrupt_file(&arch, 300_000, &[0xFF; 64]);
    assert!(arch_size > 400_000, "test tree too small for the chosen offsets");

    // Verify detects it.
    let v = run(&["verify", arch.to_str().unwrap(), "-q"]);
    fail(&v);
    assert!(stderr_of(&v).contains("corrupt"));

    // Repair with the volume.
    ok(&run(&["repair", arch.to_str().unwrap(), "--volumes", vol.to_str().unwrap(), "-f", "-q"]));
    let repaired = dir.path().join("rec.repaired.next");
    ok(&run(&["verify", repaired.to_str().unwrap(), "-q"]));

    let out = dir.path().join("out");
    ok(&run(&["extract", repaired.to_str().unwrap(), "-o", out.to_str().unwrap(), "-q"]));
    compare_trees(&src, &out.join("src"));
}

#[test]
fn recovery_repairs_truncated_archive() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(&src).unwrap();
    make_tree(&src);
    let arch = dir.path().join("t.next");
    let vol = dir.path().join("t.next.nvol");

    ok(&run(&["create", src.to_str().unwrap(), "-o", arch.to_str().unwrap(), "-r", "8", "-f", "-q"]));
    let full_size = fs::metadata(&arch).unwrap().len();

    // Cut the archive to 60%: the index (at the end) is destroyed along with
    // the tail blocks; the volume's index copy must rescue it.
    truncate_file(&arch, full_size * 6 / 10);

    ok(&run(&["repair", arch.to_str().unwrap(), "--volumes", vol.to_str().unwrap(), "-f", "-q"]));
    let repaired = dir.path().join("t.repaired.next");
    assert_eq!(fs::metadata(&repaired).unwrap().len(), full_size, "repaired archive should be full size");

    ok(&run(&["verify", repaired.to_str().unwrap(), "-q"]));
    let out = dir.path().join("out");
    ok(&run(&["extract", repaired.to_str().unwrap(), "-o", out.to_str().unwrap(), "-q"]));
    compare_trees(&src, &out.join("src"));
}

#[test]
fn encrypted_recovery_combined() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(&src).unwrap();
    make_tree(&src);
    let arch = dir.path().join("er.next");
    let vol = dir.path().join("er.next.nvol");

    ok(&run(&["create", src.to_str().unwrap(), "-o", arch.to_str().unwrap(), "-r", "8", "-p", "pw", "-f", "-q"]));
    corrupt_file(&arch, 250_000, &[0x11; 16]);
    ok(&run(&["repair", arch.to_str().unwrap(), "--volumes", vol.to_str().unwrap(), "-f", "-q"]));
    let repaired = dir.path().join("er.repaired.next");
    let out = dir.path().join("out");
    ok(&run(&["extract", repaired.to_str().unwrap(), "-o", out.to_str().unwrap(), "-p", "pw", "-q"]));
    compare_trees(&src, &out.join("src"));
}

#[test]
fn empty_dir_preserved() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    let empty = src.join("keep");
    fs::create_dir_all(&empty).unwrap();
    fs::write(src.join("f.txt"), b"x").unwrap();
    let arch = dir.path().join("e.next");
    let out = dir.path().join("out");

    ok(&run(&["create", src.to_str().unwrap(), "-o", arch.to_str().unwrap(), "-f", "-q"]));
    ok(&run(&["extract", arch.to_str().unwrap(), "-o", out.to_str().unwrap(), "-q"]));
    assert!(out.join("src/keep").is_dir(), "empty directory lost");
}

#[cfg(unix)]
#[test]
fn symlinks_preserved() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("real.txt"), b"target").unwrap();
    symlink("real.txt", src.join("link.txt")).unwrap();
    let arch = dir.path().join("s.next");
    let out = dir.path().join("out");

    ok(&run(&["create", src.to_str().unwrap(), "-o", arch.to_str().unwrap(), "-f", "-q"]));
    ok(&run(&["extract", arch.to_str().unwrap(), "-o", out.to_str().unwrap(), "-q"]));
    let link = out.join("src/link.txt");
    assert!(fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
    assert_eq!(fs::read_link(&link).unwrap().to_string_lossy().as_ref(), "real.txt");
}

#[test]
fn read_file_bytes_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(&src).unwrap();
    make_tree(&src);
    let arch = dir.path().join("rfb.next");
    ok(&run(&["create", src.to_str().unwrap(), "-o", arch.to_str().unwrap(), "-f", "-q"]));

    // single-block file
    let expected = fs::read(src.join("sub").join("nested.txt")).unwrap();
    assert_eq!(
        nextar::archive::read_file_bytes(&arch, "src/sub/nested.txt", None).unwrap(),
        expected
    );

    // multi-block file (large text)
    let big = fs::read(src.join("big.txt")).unwrap();
    assert_eq!(nextar::archive::read_file_bytes(&arch, "src/big.txt", None).unwrap(), big);

    // unknown entry / directory error cleanly
    assert!(nextar::archive::read_file_bytes(&arch, "nope.txt", None).is_err());
    assert!(nextar::archive::read_file_bytes(&arch, "src", None).is_err());
}

#[test]
fn read_file_bytes_encrypted() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("secret.txt"), b"top secret payload").unwrap();
    let arch = dir.path().join("enc.next");
    ok(&run(&["create", src.to_str().unwrap(), "-o", arch.to_str().unwrap(), "-p", "hunter2", "-f", "-q"]));

    let expected = fs::read(src.join("secret.txt")).unwrap();
    assert_eq!(
        nextar::archive::read_file_bytes(&arch, "src/secret.txt", Some("hunter2")).unwrap(),
        expected
    );
    // wrong or missing password on an encrypted archive → clean error
    assert!(nextar::archive::read_file_bytes(&arch, "src/secret.txt", Some("nope")).is_err());
    assert!(nextar::archive::read_file_bytes(&arch, "src/secret.txt", None).is_err());
}

#[test]
fn wrong_password_reported_cleanly() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("a.txt"), b"data").unwrap();
    let arch = dir.path().join("p.next");
    ok(&run(&["create", src.to_str().unwrap(), "-o", arch.to_str().unwrap(), "-p", "secret", "-f", "-q"]));
    let out = run(&["extract", arch.to_str().unwrap(), "-o", dir.path().join("o").to_str().unwrap(), "-p", "nope", "-q"]);
    fail(&out);
    assert!(stderr_of(&out).contains("wrong password"));
}
