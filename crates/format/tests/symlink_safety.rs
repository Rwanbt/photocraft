//! Directory .pcraft bundles must never redirect writes/deletes outside their chosen root.
//! A malicious or corrupted bundle can contain pre-existing symlinks.
#![cfg(unix)]

mod common;

use common::{rich_doc, temp_dir};
use photocraft_color::{ColorMode, SampleType};
use photocraft_format::{PcraftWriter, SaveOptions};
use std::os::unix::fs::symlink;

fn sample_doc() -> photocraft_doc::Document {
    rich_doc(ColorMode::Rgb, SampleType::U8)
}

fn assert_refused(error: photocraft_format::FormatError) {
    let message = error.to_string();
    assert!(message.contains("symbolic link"), "unexpected error: {message}");
}

#[test]
fn directory_component_symlinks_never_redirect_bundle_saves() {
    let doc = sample_doc();
    for component in ["tiles", "blobs", "composite"] {
        let root = temp_dir("dir-link");
        let bundle = root.join("bundle");
        let outside = root.join("outside");
        std::fs::create_dir_all(&bundle).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let sentinel = outside.join("sentinel.zst");
        std::fs::write(&sentinel, b"do not touch external files").unwrap();
        symlink(&outside, bundle.join(component)).unwrap();

        let result = PcraftWriter::new().save_dir(&doc, &bundle, &SaveOptions::default());
        assert_refused(result.expect_err("bundle with internal directory symlink must fail"));
        assert_eq!(std::fs::read(&sentinel).unwrap(), b"do not touch external files");
        assert!(!bundle.join("manifest.json").exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn manifest_and_preview_symlinks_never_redirect_or_remove_external_files() {
    let doc = sample_doc();
    for component in ["manifest.json", "thumb.png", "composite/preview.png"] {
        let root = temp_dir("file-link");
        let bundle = root.join("bundle");
        std::fs::create_dir_all(bundle.join("composite")).unwrap();
        let outside = root.join("external.data");
        std::fs::write(&outside, b"original contents").unwrap();
        symlink(&outside, bundle.join(component)).unwrap();

        let result = PcraftWriter::new().save_dir(&doc, &bundle, &SaveOptions::default());
        assert_refused(result.expect_err("bundle with linked manifest or preview must fail"));
        assert_eq!(std::fs::read(&outside).unwrap(), b"original contents");
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn individual_tile_symlink_cannot_overwrite_external_destination() {
    let doc = sample_doc();
    let root = temp_dir("object-link");
    let bundle = root.join("bundle");
    PcraftWriter::new().save_dir(&doc, &bundle, &SaveOptions::default()).unwrap();

    let tile = std::fs::read_dir(bundle.join("tiles"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.extension().is_some_and(|ext| ext == "zst"))
        .expect("sample has tile objects");
    let outside = root.join("private-backup.bin");
    std::fs::write(&outside, b"untouched private bytes").unwrap();
    std::fs::remove_file(&tile).unwrap();
    symlink(&outside, &tile).unwrap();

    let result = PcraftWriter::new().save_dir(&doc, &bundle, &SaveOptions::default());
    assert_refused(result.expect_err("symlinked content-addressed object must fail"));
    assert_eq!(std::fs::read(&outside).unwrap(), b"untouched private bytes");
    std::fs::remove_dir_all(root).unwrap();
}
